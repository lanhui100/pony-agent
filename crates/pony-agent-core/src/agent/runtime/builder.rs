use super::*;

pub struct AgentRuntimeBuilder {
    sessions: Option<SessionStore>,
    provider_resolver: Option<Box<dyn ProviderSelectionResolver>>,
    tool_executor: Option<Box<dyn ToolExecutor>>,
    workspace_root: Option<std::path::PathBuf>,
    planner: Option<Box<dyn TurnPlanner>>,
    context_builder: Option<Box<dyn TurnContextBuilder>>,
    telemetry_builder: Option<Box<dyn TurnTelemetryBuilder>>,
}


impl AgentRuntimeBuilder {
    pub fn new() -> Self {
        Self {
            sessions: None,
            provider_resolver: None,
            tool_executor: None,
            workspace_root: None,
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

    /// Seeds the workspace root used by the default (governed) tool executor. This does not
    /// select a specific executor: an explicit `tool_executor(...)` override still wins, and the
    /// default engine is `build_governed_executor(Some(root))` (PA-076 runtime switch).
    pub fn workspace_root(mut self, workspace_root: impl Into<std::path::PathBuf>) -> Self {
        self.workspace_root = Some(workspace_root.into());
        self
    }

    pub fn build(self) -> AgentRuntime {
        let sessions = self.sessions.unwrap_or_else(SessionStore::new);
        let shared_authorizations = sessions.path_authorizations();
        let mut runtime = AgentRuntime::with_dependencies(
            sessions,
            self.provider_resolver
                .unwrap_or_else(|| Box::new(ProviderRegistryStore::new())),
            self.tool_executor.unwrap_or_else(|| {
                Box::new(build_governed_executor(
                    self.workspace_root.clone(),
                    Some(shared_authorizations),
                ))
            }),
            self.planner.unwrap_or_else(|| Box::new(LocalTurnPlanner)),
            self.context_builder
                .unwrap_or_else(|| Box::new(DefaultTurnContextBuilder)),
            self.telemetry_builder
                .unwrap_or_else(|| Box::new(DefaultTurnTelemetryBuilder)),
        );
        runtime.workspace_root = self
            .workspace_root
            .map(|path| path.display().to_string());
        runtime
    }
}


impl Default for AgentRuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}


pub struct DesktopRuntimePreset;


impl DesktopRuntimePreset {
    pub fn builder() -> AgentRuntimeBuilder {
        AgentRuntimeBuilder::desktop()
    }

    pub fn build() -> AgentRuntime {
        Self::builder().build()
    }
}
