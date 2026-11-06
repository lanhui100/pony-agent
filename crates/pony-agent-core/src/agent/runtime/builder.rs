use super::*;

pub struct AgentRuntimeBuilder {
    sessions: Option<SessionStore>,
    /// W3b：外部注入的**共享**会话存储（优先于自建 `sessions`）。宿主注入后，由本 builder
    /// 注册的 [`HostTurnEngine`] 跑队友 turn 时复用同一份 store ⇒ 队友会话对宿主可见。
    sessions_shared: Option<Arc<RwLock<SessionStore>>>,
    provider_resolver: Option<Box<dyn ProviderSelectionResolver>>,
    tool_executor: Option<Box<dyn ToolExecutor>>,
    workspace_root: Option<std::path::PathBuf>,
    planner: Option<Box<dyn TurnPlanner>>,
    context_builder: Option<Box<dyn TurnContextBuilder>>,
    telemetry_builder: Option<Box<dyn TurnTelemetryBuilder>>,
    plan_store: Option<crate::agent::plan_state::PlanStore>,
    /// wave-65 C1：审批挂起可恢复能力位（默认 `false`，仅 TUI 宿主打开）。
    host_approval_resume: bool,
}


impl AgentRuntimeBuilder {
    pub fn new() -> Self {
        Self {
            sessions: None,
            sessions_shared: None,
            provider_resolver: None,
            tool_executor: None,
            workspace_root: None,
            planner: None,
            context_builder: None,
            telemetry_builder: None,
            plan_store: None,
            host_approval_resume: false,
        }
    }

    pub fn desktop() -> Self {
        Self::new()
    }

    pub fn session_store(mut self, sessions: SessionStore) -> Self {
        self.sessions = Some(sessions);
        self
    }

    /// W3b：注入**共享**会话存储（与宿主 control plane 同一 `Arc`）。
    ///
    /// 优先级：本方法注入的 Arc > [`AgentRuntimeBuilder::session_store`] 传入的自有 store >
    /// 默认 `SessionStore::new()`。注入后 `build()` 会把**同一个** Arc 交给注册的
    /// [`HostTurnEngine`]，队友 turn 的会话落库因此直接可见于宿主 `list_sessions()`。
    pub fn shared_session_store(
        mut self,
        sessions: Arc<RwLock<SessionStore>>,
    ) -> Self {
        self.sessions_shared = Some(sessions);
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

    pub fn plan_store(mut self, plan_store: crate::agent::plan_state::PlanStore) -> Self {
        self.plan_store = Some(plan_store);
        self
    }

    /// wave-65 C1：打开/关闭「审批（`Approval`）挂起可恢复」能力位。
    ///
    /// 默认 `false` —— 桌面（`src-tauri`）与 CLI 不调用本方法，行为逐字不变（挂起仍
    /// fail-fast `approval_resume_unavailable`）。唯一消费点是 TUI 宿主建栈。
    pub fn host_approval_resume(mut self, enabled: bool) -> Self {
        self.host_approval_resume = enabled;
        self
    }

    pub fn build(self) -> AgentRuntime {
        // Wave-61 C2：宿主子代理引擎的**唯一**注册点（三条宿主 tauri / non-tauri harness / tui
        // 以及 cli 全部经由本函数建栈，见契约 C2.1）。把注册收敛在这里，宿主侧零改动。
        //
        // 幂等与不阻断：槽是 `OnceLock`（subagent_runtime.rs:37-45），首写者胜、重复注册返回
        // `Err("subagent engine already set")`；`build()` 返回 `AgentRuntime` 而非 `Result`，
        // 故注册失败（含已注册、含 HostTurnEngine 自建栈的嵌套 build）一律忽略，**不** panic、
        // **不**改变返回值。公共 API `set_subagent_engine` / `subagent_engine` 语义不变。
        // W3b：统一得到一个 Arc —— 注入的共享 store 优先，否则把自有 `sessions` 包成 Arc。
        // 该 Arc 同时用于：① 本 runtime 的会话句柄（宿主 control plane 经 `sessions_handle()`
        // 读取）；② 注册给 `HostTurnEngine`（队友 turn 复用同一份 store）。
        let sessions_arc = match (self.sessions_shared, self.sessions) {
            (Some(shared), _) => shared,
            (None, Some(sessions)) => Arc::new(RwLock::new(sessions)),
            (None, None) => Arc::new(RwLock::new(SessionStore::new())),
        };
        ensure_host_subagent_engine(
            self.workspace_root.clone(),
            Some(Arc::clone(&sessions_arc)),
        );
        let shared_authorizations = sessions_arc
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .path_authorizations();
        let plan_store = self.plan_store;
        let mut runtime = AgentRuntime::with_shared_sessions(
            sessions_arc,
            self.provider_resolver
                .unwrap_or_else(|| Box::new(ProviderRegistryStore::new())),
            self.tool_executor.unwrap_or_else(|| {
                Box::new(crate::agent::governed_executor::build_governed_executor_with_plan_store(
                    self.workspace_root.clone(),
                    Some(shared_authorizations),
                    plan_store,
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
        // wave-65 C1：能力位透传（默认 `false` ⇒ 既有宿主行为逐字不变）。
        runtime.host_approval_resume = self.host_approval_resume;
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

/// Wave-61 C2：把宿主子代理引擎（[`HostTurnEngine`]）幂等注册进进程级槽。
///
/// 语义约束（契约 C2.2）：
/// - 传**本 builder 的** `workspace_root`（`with_workspace_root`），满足 wave-58 F1-5「子代理必须
///   以父会话工作区根建栈」；`None` 时引擎按其既有规则回落 `compute_default_workspace_root()`。
/// - 槽已接线（含本进程内更早的 `build()`、或测试/宿主经公开 API 显式注入）时**不覆盖**，
///   首写者胜是该 `OnceLock` 的既定语义。
/// - 注册失败（`Err("subagent engine already set")` 等）一律忽略：`build()` 不返回 `Result`，
///   不得 panic、不得改变返回值、不得阻断宿主启动。
/// - W3b：`shared_sessions` = 本 builder 最终使用的会话存储 Arc。首写者胜意味着「先建栈的 runtime」
///   决定队友引擎的共享 store —— 宿主（TUI/tauri/harness/cli）都是首个 builder，因此队友会话
///   落在宿主 store 里；后续嵌套 build（含队友自身建栈）不会覆盖既有引擎。
fn ensure_host_subagent_engine(
    workspace_root: Option<std::path::PathBuf>,
    shared_sessions: Option<Arc<RwLock<SessionStore>>>,
) {
    use crate::agent::subagent_runtime::{
        set_subagent_engine, subagent_engine, HostTurnEngine,
    };
    if subagent_engine().is_some() {
        return;
    }
    let engine = HostTurnEngine::new()
        .with_workspace_root(workspace_root)
        .with_shared_sessions(shared_sessions);
    let _ = set_subagent_engine(std::sync::Arc::new(engine));
}
