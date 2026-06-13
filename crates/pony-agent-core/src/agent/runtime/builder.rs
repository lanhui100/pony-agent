use crate::agent::config::{ProviderRegistryStore, ProviderSelectionResolver};
use crate::agent::context::DefaultTurnContextBuilder;
use crate::agent::planner::LocalTurnPlanner;
use crate::agent::runtime::{AgentRuntime, AgentRuntimeBuilder};
use crate::agent::session::SessionStore;
use crate::agent::telemetry::TurnTelemetryBuilder;
use crate::agent::tools::{ToolExecutor, ToolRouter};

impl AgentRuntimeBuilder {
    pub fn new() -> Self {
        Self {
            sessions: None,
            provider_resolver: None,
            tool_executor: None,
            planner: None,
            context_builder: None,
            telemetry_builder: None,
        }
    }

    pub fn desktop() -> Self {
        Self::new()
    }

    pub fn session_store(mut self, sessions: SessionStore) -> Self {
        self.sessions = Some(sessions);
        self
    }

    pub fn provider_resolver(
        mut self,
        provider_resolver: Box<dyn ProviderSelectionResolver>,
    ) -> Self {
        self.provider_resolver = Some(provider_resolver);
        self
    }

    pub fn tool_executor(mut self, tool_executor: Box<dyn ToolExecutor>) -> Self {
        self.tool_executor = Some(tool_executor);
        self
    }

    pub fn planner(mut self, planner: Box<dyn TurnPlanner>) -> Self {
        self.planner = Some(planner);
        self
    }

    pub fn context_builder(mut self, context_builder: Box<dyn TurnContextBuilder>) -> Self {
        self.context_builder = Some(context_builder);
        self
    }

    pub fn telemetry_builder(mut self, telemetry_builder: Box<dyn TurnTelemetryBuilder>) -> Self {
        self.telemetry_builder = Some(telemetry_builder);
        self
    }

    pub fn workspace_root(mut self, workspace_root: impl Into<std::path::PathBuf>) -> Self {
        self.tool_executor = Some(Box::new(ToolRouter::with_workspace_root(
            workspace_root.into(),
        )));
        self
    }

    pub fn build(self) -> AgentRuntime {
        AgentRuntime::with_dependencies(
            self.sessions.unwrap_or_else(SessionStore::new),
            self.provider_resolver
                .unwrap_or_else(|| Box::new(ProviderRegistryStore::new())),
            self.tool_executor
                .unwrap_or_else(|| Box::new(ToolRouter::new())),
            self.planner.unwrap_or_else(|| Box::new(LocalTurnPlanner)),
            self.context_builder
                .unwrap_or_else(|| Box::new(DefaultTurnContextBuilder)),
            self.telemetry_builder
                .unwrap_or_else(|| Box::new(DefaultTurnTelemetryBuilder)),
        )
    }
}


impl Default for AgentRuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience preset that builds a desktop-grade runtime with default
/// dependencies. Useful for tests and lightweight embedding.
pub struct DesktopRuntimePreset;

impl DesktopRuntimePreset {
    pub fn builder() -> AgentRuntimeBuilder {
        AgentRuntimeBuilder::desktop()
    }

    pub fn build() -> AgentRuntime {
        Self::builder().build()
    }
}