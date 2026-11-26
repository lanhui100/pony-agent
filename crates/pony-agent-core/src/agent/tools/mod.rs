pub use pony_tools::tools::*;
pub use crate::agent::orchestration::{
    drain_inbox, interrupt_agent, interrupt_agent_as, list_agents, send_message,
    send_message_in_workspace, wait_agent_as, TeamCaller, spawn_teammate,
    spawn_teammate_in_workspace, spawn_teammate_in_workspace_with_provider, subagent,
    subagent_in_workspace, subagent_in_workspace_with_context, subagent_in_workspace_with_provider,
    team_task_create, team_task_get, team_task_list, team_task_update, wait_agent, workflow,
    InterruptAgentArgs, InterruptAgentResult, ListAgentsResult, MessageDeliveryMode, QueuedMessage,
    ReadInboxArgs, ReadInboxResult, SendMessageArgs, SendMessageResult, SpawnTeammateArgs,
    SpawnTeammateResult, SubagentArgs, SubagentResult, TeamTaskAction, TeamTaskCreateArgs,
    TeamTaskGetArgs, TeamTaskItem, TeamTaskListResult, TeamTaskUpdateArgs, TeammateMember,
    TeammateStatus, WaitAgentArgs, WaitAgentNoProgress, WaitAgentResult, WorkflowArgs,
    WorkflowResult, WorkflowStep,
};
#[cfg(test)]
#[path = "../tools_orchestration_tests.rs"]
mod orchestration_tests;

pub struct ToolRouter(pub pony_tools::tools::ToolRouter);

impl std::ops::Deref for ToolRouter {
    type Target = pony_tools::tools::ToolRouter;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ToolRouter {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl ToolRouter {
    pub fn new() -> Self {
        crate::agent::ensure_runtime_bridges();
        Self(pony_tools::tools::ToolRouter::new())
    }

    pub fn with_workspace_root(workspace_root: std::path::PathBuf) -> Self {
        crate::agent::ensure_runtime_bridges();
        Self(pony_tools::tools::ToolRouter::with_workspace_root(workspace_root))
    }

    pub fn with_authorize_store(
        mut self,
        authorize_store: std::sync::Arc<crate::agent::path_permission::AuthorizeStore>,
    ) -> Self {
        self.0 = self.0.with_authorize_store(authorize_store);
        self
    }

    pub fn with_root_resolver<F>(mut self, resolver: F) -> Self
    where
        F: Fn(&str) -> Option<std::path::PathBuf> + Send + Sync + 'static,
    {
        self.0 = self.0.with_root_resolver(resolver);
        self
    }

    pub fn with_sandbox_backend<B: crate::agent::tool_runtime::SandboxBackend + 'static>(
        mut self,
        backend: B,
    ) -> Self {
        self.0 = self.0.with_sandbox_backend(backend);
        self
    }

    pub fn with_web_resolver<R: crate::agent::tool_runtime::WebResolver + 'static>(
        mut self,
        resolver: R,
    ) -> Self {
        self.0 = self.0.with_web_resolver(resolver);
        self
    }
}

impl Default for ToolRouter {
    fn default() -> Self {
        Self::new()
    }
}

impl ToolExecutor for ToolRouter {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        self.0.execute(call)
    }

    fn execute_with_context(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        self.0.execute_with_context(call, context)
    }
}
