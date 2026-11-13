use crate::agent::capability_bridge::{
    enrich_mcp_source_snapshot, enrich_skill_source_snapshot, CapabilityFailureKind,
    CapabilityRegistry, CapabilityToolExecutionResult, McpSourceSnapshot, SkillFailureLayer,
    SkillInvocationRequest, SkillSourceSnapshot, SkillToolExecutionResult,
};
use crate::agent::config::{
    ProviderReasoningEffort, ProviderRegistryStore, ProviderSelectionResolver,
};
use crate::agent::context::{DefaultTurnContextBuilder, RetrievedContextState, TurnContextBuilder};
use crate::agent::dispatcher::{DispatchContext, GovernedDispatcher};
use crate::agent::dispatcher_composites::GovernedToolExecutor;
use crate::agent::execution_control::ExecutionCheckpoint;
use crate::agent::execution_control::ExecutionControlRegistry;
#[cfg_attr(not(test), allow(unused_imports))]
pub(super) use crate::agent::governed_executor::build_governed_executor;
use crate::agent::graph::{
    GraphAskResumeInjection, GraphAskWaitBinding, GraphDecision, GraphDecisionKind, GraphEngine,
    GraphRun, GraphRunStore, GraphRunner, GraphTurnHandoff,
};
use crate::agent::hooks::{
    build_observe_hook_trace_record, turn_hook_point_for_capability_mediation_hook_point,
    turn_hook_point_for_planner_hook_point, AgentHookDescriptor, AgentHookExecutor,
    AgentHookRegistry, CapabilityMediationEnvelope, CapabilityMediationHookPoint,
    HookFailurePolicy, HookPatchConflictPolicy, HookPatchOperation, HookPatchOperationKind,
    HookStructuredResult, HookTraceRecord, PlannerFactsEnvelope,
    PlannerHookPoint, TurnHookPoint,
};
use crate::agent::input::TurnInputImage;
use crate::agent::planner::{GraphPlanner, LocalTurnPlanner, TurnPlanner};
use crate::agent::provider::{
    build_context_observation, provider_native_assistant_message_with_reasoning,
    provider_native_assistant_message_with_reasoning_value,
    provider_native_assistant_tool_call_message,
    provider_native_assistant_tool_call_message_for_protocol,
    provider_native_assistant_tool_call_message_with_reasoning_value,
    provider_native_tool_result_message_for_protocol, provider_native_user_message,
    BuildContextObservation, ProviderDecision, ProviderManager, ProviderMessage, ProviderRequest,
    ProviderRequestObservation, ProviderResponse, ProviderStreamChunk, TokenUsage,
};
use crate::agent::session::SessionTraceMutation;
use crate::agent::session::{
    HistoryBranch, HistoryCheckoutMode, HistoryCursor, HistoryMessageKind, HistoryNode,
    SessionAttachment, SessionOverview, SessionSnapshot, SessionStore, TraceTimelineEntry,
    TurnHistoryMessage, TurnTraceRecord,
};
use crate::agent::telemetry::{
    DefaultTurnTelemetryBuilder, ProviderCallCacheRecord, ProviderLatencyKind, ProviderRequestKind,
    TurnTelemetryBuilder, TurnToolActivity, TurnTraceStep,
};
use std::path::PathBuf;
use crate::agent::tools::{
    canonical_tool_name, default_permission_facts_for_name, tool_error_from_output,
    ToolCall, ToolDefinition, ToolExecutionContext, ToolExecutor, ToolResult,
};
use crate::agent::tool_runtime::{
    PendingControlRequest, PendingControlRequestKind, PendingControlRequestState,
};
use crate::agent::trace_persistence::{
    spawn_trace_persistence_worker, TracePersistenceCommand, TracePersistenceHandle,
};
use crate::agent::turn_flow::{
    build_failed_turn_result, build_failed_turn_result_with_hooks,
    emit_stream_cancelled, emit_stream_event, emit_stream_event_with_step,
    emit_stream_failed, emit_turn_failed, normalize_user_message, preview_text,
    provider_decision_stream_guarded_with_mode, provider_decision_with_mode,
    provider_event_meta, provider_failure_message, provider_followup_stream_with_calls,
    provider_followup_stream_with_mode,
    provider_followup_with_calls, provider_followup_with_mode, runtime_log,
    stream_reasoning_chunks, stream_text_chunks,
    token_usage_parts, ModelHopTraceContent, PersistedTurnOutcome, PlannedTurn, PreparedTurn,
    ProviderEventMeta, SyncToolTurnOutcome, TurnEventEnvelope, TurnEventSink,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;

pub mod turn_runner;

mod compaction_ops;
mod ingress_mediation;
mod session_ops;
mod tool_exec;
mod turn_persist;
mod turn_prep;

mod batch_plan;
mod blocked_records;
mod builder;
mod limits;
mod planner_patches;
mod stream_support;
mod tool_recovery;
mod trace_timeline;
mod turn_control;
mod turn_events;
mod turn_sync;
mod turn_stream;
mod turn_stream_failures;
mod types;

pub use builder::{AgentRuntimeBuilder, DesktopRuntimePreset};
// F4：错误码单一定义点。宿主（bin/tui.rs 是同包独立 crate root）必须引用该常量拼文案，
// 不得再手写字面量（否则 `docs/observability.yaml` 的 error_classifier 收口点会出现第二定义点）。
pub use turn_control::ASK_RESUME_UNAVAILABLE_CODE;
pub use compaction_ops::{compact_outcome_message, ManualCompactionOutcome};
pub(crate) use compaction_ops::compact_session_with_sessions;
pub use types::{PlannerGraphDecisionDispatchOutcome, RunTurnFacts, SUSPENDED_TURN_PHASE, TurnInput, TurnResult, TurnStreamEvent};

/// Which store key a peeked Ask injection came from (run-scoped vs session-scoped).
#[derive(Clone, Debug)]
pub(crate) enum AskInjectionKey {
    Run(String),
    Session(String),
}

/// A peeked (not yet consumed) Ask injection lease. The runtime acks it only after a successful
/// terminal state; dropping the lease without ack leaves the injection queued (contract §5).
#[derive(Clone, Debug)]
pub(crate) struct AskInjectionLease {
    key: AskInjectionKey,
    pub(crate) request_id: String,
    pub(crate) call_id: String,
    pub(crate) injection: GraphAskResumeInjection,
}

use batch_plan::*;
use blocked_records::*;
use limits::*;
pub use limits::{max_tool_followups_per_turn, parse_max_tool_followups_per_turn};
pub use stream_support::tool_failure_signal;
/// wave-54 C4：协议归一上下文输入（turn_flow 的 span 装配面共用同一实现）。
pub(crate) use stream_support::context_input_tokens_for;
use planner_patches::*;
use stream_support::*;
use tool_recovery::*;
use trace_timeline::*;
use turn_events::*;


// 原 pub(crate) 项的 crate 级寻址契约保持（reviewer-b P2-1 后果二修复）
pub(crate) use planner_patches::{apply_capability_argument_patches, apply_planner_patches, normalized_arguments_from_summary};
pub(crate) use types::{CapabilityMediationDispatchOutcome, GraphDecisionDispatchOutcome, HookDispatchOutcome, PlannerDispatchOutcome};
use types::*;

pub struct AgentRuntime {
    graph: GraphEngine,
    sessions: Arc<RwLock<SessionStore>>,
    provider_resolver: Box<dyn ProviderSelectionResolver>,
    capability_registry: Arc<RwLock<CapabilityRegistry>>,
    hook_registry: AgentHookRegistry,
    hook_executor: Box<dyn AgentHookExecutor>,
    tool_executor: Box<dyn ToolExecutor>,
    planner: Box<dyn TurnPlanner>,
    context_builder: Box<dyn TurnContextBuilder>,
    telemetry_builder: Box<dyn TurnTelemetryBuilder>,
    /// 后台 trace 落库句柄：可观测性写盘与主对话执行路径解耦。
    trace_persistence: Option<TracePersistenceHandle>,
    /// Shared graph run store backing Ask-wait suspension binding (design.md Decision 5). The
    /// host control plane shares the same store so `graph_bind_ask_wait` / `graph_resume_ask`
    /// observe the binding the runtime creates. `None` when the runtime runs outside a graph.
    graph_runs: Option<Arc<Mutex<GraphRunStore>>>,
    /// Workspace root facts seeded into the governed `DispatchContext` per turn.
    workspace_root: Option<String>,
    /// W3 trace：本 runtime 的 turn 要继承的宿主 `traceparent`（`None` = 自起 root trace）。
    ///
    /// 由 `HostTurnEngine` 建栈后按派发时的宿主 traceparent 注入：teammate/subagent turn 的
    /// `agent.turn` span 因此挂在宿主当前 turn span 之下（同一 trace id）。
    parent_traceparent: Option<String>,
    /// The governed dispatcher the default tool executor executes through, cached once so the
    /// runtime and the host control plane share the exact same `Arc` (design.md Decision 5,
    /// P1-1). Computed from the tool executor at first use; `None` when the executor is not the
    /// governed adapter.
    governed_dispatcher: OnceLock<Option<Arc<GovernedDispatcher>>>,
    /// wave-65 C1：宿主是否具备「审批（`Approval`）挂起可恢复」能力（唯一开关）。
    ///
    /// 默认 `false` ⇒ 桌面（`src-tauri`）/ CLI 行为逐字不变（仍 fail-fast
    /// `approval_resume_unavailable`）。只有 TUI 侧经
    /// `AgentRuntimeBuilder::host_approval_resume(true)` 打开。
    host_approval_resume: bool,
}

impl AgentRuntime {
    pub fn new() -> Self {
        DesktopRuntimePreset::build()
    }

    /// 当前 workspace root（PA-078 宿主 `get_workspace_root` 读取）。
    pub fn workspace_root(&self) -> Option<&str> {
        self.workspace_root.as_deref()
    }

    /// 以自有会话存储建 runtime（行为与既有调用方完全一致）：内部包成 `Arc<RwLock<_>>` 后
    /// 委托 [`AgentRuntime::with_shared_sessions`]。
    pub fn with_dependencies(
        sessions: SessionStore,
        provider_resolver: Box<dyn ProviderSelectionResolver>,
        tool_executor: Box<dyn ToolExecutor>,
        planner: Box<dyn TurnPlanner>,
        context_builder: Box<dyn TurnContextBuilder>,
        telemetry_builder: Box<dyn TurnTelemetryBuilder>,
    ) -> Self {
        Self::with_shared_sessions(
            Arc::new(RwLock::new(sessions)),
            provider_resolver,
            tool_executor,
            planner,
            context_builder,
            telemetry_builder,
        )
    }

    /// W3b：以**共享**会话存储建 runtime。
    ///
    /// 宿主与队友引擎传入同一个 `Arc<RwLock<SessionStore>>` 时，队友 turn 的落库直接写进宿主
    /// control plane 读取的同一份表 ⇒ 队友会话立刻出现在 `list_sessions()`，`/agents` 的
    /// Enter 才具备 `hydrate` 前置条件（此前的独立 store 永远不可加载）。
    pub fn with_shared_sessions(
        sessions: Arc<RwLock<SessionStore>>,
        provider_resolver: Box<dyn ProviderSelectionResolver>,
        tool_executor: Box<dyn ToolExecutor>,
        planner: Box<dyn TurnPlanner>,
        context_builder: Box<dyn TurnContextBuilder>,
        telemetry_builder: Box<dyn TurnTelemetryBuilder>,
    ) -> Self {
        let (persisted_mcp_snapshots, persisted_skill_snapshots) = {
            let store = sessions.read().unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            });
            (
                store.list_persisted_mcp_source_snapshots(),
                store.list_persisted_skill_source_snapshots(),
            )
        };
        let mut capability_registry = CapabilityRegistry::new();
        for snapshot in persisted_mcp_snapshots {
            capability_registry.replace_mcp_source_snapshot(snapshot);
        }
        for snapshot in persisted_skill_snapshots {
            let _ = capability_registry.replace_skill_source_snapshot(snapshot);
        }
        // W30a：钩子注册表与执行器必须同时构建——只给执行器不给描述符会导致钩子点永远不触发。
        let (hook_registry, hook_executor) =
            crate::agent::hook_commands::build_default_hooks();
        let trace_persistence = Some(spawn_trace_persistence_worker(Arc::clone(&sessions)));
        Self {
            graph: GraphEngine::new("state-machine-v1"),
            sessions,
            provider_resolver,
            capability_registry: Arc::new(RwLock::new(capability_registry)),
            hook_registry,
            hook_executor,
            tool_executor,
            planner,
            context_builder,
            telemetry_builder,
            trace_persistence,
            graph_runs: None,
            workspace_root: None,
            parent_traceparent: None,
            governed_dispatcher: OnceLock::new(),
            host_approval_resume: false,
        }
    }

    pub fn sessions_handle(&self) -> Arc<RwLock<SessionStore>> {
        Arc::clone(&self.sessions)
    }

    /// W3 trace：注入本 runtime 各 turn 要继承的宿主 `traceparent`（`None` = 保持 root trace）。
    ///
    /// 语义：写入后**每一次** turn 的 `agent.turn` span 都以该父上下文起建（同一 trace id）。
    /// 仅在队友/子代理引擎的独立 runtime 上调用；宿主 runtime 保持 `None`（原有行为不变）。
    pub fn set_parent_traceparent(&mut self, parent_traceparent: Option<String>) {
        self.parent_traceparent = parent_traceparent;
    }

    /// Share the graph run store used for Ask-wait suspension binding (design.md Decision 5).
    /// The host control plane injects the same `Arc<Mutex<GraphRunStore>>` it reads/writes for
    /// `graph_bind_ask_wait` / `graph_resume_ask`, so a binding the runtime creates during a turn
    /// is immediately visible to the host surface.
    pub fn set_graph_run_store(&mut self, store: Arc<Mutex<GraphRunStore>>) {
        self.graph_runs = Some(store);
    }

    pub fn graph_run_store(&self) -> Option<Arc<Mutex<GraphRunStore>>> {
        self.graph_runs.clone()
    }

    /// Peek (do **not** consume) the one-shot Ask resume injection for this turn.
    ///
    /// Contract §5: injection consumption is `peek → provider turn → ack on a successful terminal
    /// state`. A destructive take here would lose the answer whenever prepare/provider fails
    /// (L3-P `inject-drop`). Run-scoped injections (bound graph wait) win over session-scoped ones.
    fn peek_pending_ask_injection(
        &self,
        run_id: Option<&str>,
        session_id: Option<&str>,
    ) -> Option<AskInjectionLease> {
        let store_arc = self.graph_run_store()?;
        let store = store_arc.lock().unwrap_or_else(|e| {
            tracing::warn!(target: "control_plane", error = %e, "[pony-agent] graph run store poisoned");
            e.into_inner()
        });
        if let Some(run_id) = run_id {
            if let Some(injection) = store.peek_ask_injection(run_id) {
                return Some(AskInjectionLease {
                    key: AskInjectionKey::Run(run_id.to_string()),
                    request_id: injection.request_id.clone(),
                    call_id: injection.call_id.clone(),
                    injection,
                });
            }
        }
        let session_id = session_id?;
        let injection = store.peek_ask_injection_for_session(session_id)?;
        runtime_log(format!(
            "turn:ask-injection source=session request={} call={}",
            injection.request_id, injection.call_id
        ));
        Some(AskInjectionLease {
            key: AskInjectionKey::Session(session_id.to_string()),
            request_id: injection.request_id.clone(),
            call_id: injection.call_id.clone(),
            injection,
        })
    }

    /// Ack a peeked Ask injection: only called on a **successful terminal** state
    /// (`completed` / `suspended`). Failed/cancelled turns keep the injection for the next turn.
    fn ack_pending_ask_injection(&self, lease: &AskInjectionLease, reason: &str) {
        let Some(store_arc) = self.graph_run_store() else {
            return;
        };
        let mut store = store_arc.lock().unwrap_or_else(|e| {
            tracing::warn!(target: "control_plane", error = %e, "[pony-agent] graph run store poisoned");
            e.into_inner()
        });
        let acked = match &lease.key {
            AskInjectionKey::Run(run_id) => {
                store.ack_ask_injection(run_id, &lease.request_id)
            }
            AskInjectionKey::Session(session_id) => {
                store.ack_ask_injection_for_session(session_id, &lease.request_id)
            }
        };
        if acked {
            runtime_log(format!(
                "turn:ask-injection-acked request={} call={} reason={reason}",
                lease.request_id, lease.call_id
            ));
        }
    }

    /// The governed dispatcher the default tool executor executes through, if the runtime's
    /// `ToolExecutor` is the governed adapter. The host control plane shares this exact `Arc` so
    /// `ask_answer` / `ask_cancel` hit the same pending-request store the runtime persists Ask
    /// control requests into (design.md Decision 5, P1-1). Cached on first use; the same `Arc`
    /// is returned for the lifetime of the runtime.
    pub fn governed_dispatcher(&self) -> Option<Arc<GovernedDispatcher>> {
        self.governed_dispatcher
            .get_or_init(|| {
                self.tool_executor
                    .as_any()?
                    .downcast_ref::<GovernedToolExecutor>()
                    .map(|governed| Arc::new(governed.dispatcher().clone()))
            })
            .clone()
    }

    /// 解析本 turn 的治理事实（session/run/turn/workspace/授权模式/审批证据），作为
    /// `ToolExecutionContext.dispatch_context` 的载荷随**每次工具调用**显式下传（T2-1）。
    ///
    /// 返回值 `None` 表示本 runtime 的工具执行器不是治理适配器（无需注入）。本函数顺带完成
    /// 会话的 workspace 归属盖章（每 turn 一次）；它**不再**把事实写进 executor 的共享槽位 ——
    /// 那正是并发 turn 互相覆盖的根源。
    ///
    /// `None` run/turn 事实（plain `run_turn`）仍绑定会话；`host_control_available` 保持 `true`，
    /// 以便交互式宿主可以应答控制请求。
    pub(crate) fn resolve_governed_turn_context(
        &self,
        input: &TurnInput,
        facts: &RunTurnFacts,
    ) -> Option<DispatchContext> {
        // 三级树 #3（确定性根解析）：先按 input 盖章/归一（未注册 id 由 stamp
        // 归一为 default），随后一律以"盖章后的会话权威归属"解析工具根——
        // 前端 id 漂移/旧列表快照对后端免疫，杜绝"会话在新工作区、工具在默认 root"。
        let stamped_owner: Option<String> = 'stamp: {
            let Some(session_id) = input.session_id.as_deref() else {
                break 'stamp None;
            };
            let mut sessions = self.sessions.write().unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            });
            if let Some(workspace_id) = input
                .workspace_id
                .as_deref()
                .filter(|value| !value.trim().is_empty())
            {
                sessions.stamp_workspace_id(session_id, workspace_id);
            }
            break 'stamp sessions.read_workspace_owner(session_id);
        };
        let governed = self
            .tool_executor
            .as_any()
            .and_then(|any| any.downcast_ref::<GovernedToolExecutor>());
        if governed.is_none() {
            return None;
        }
        // PA-096 Phase 1：会话级 workspace root 解析——严格 Fail-closed 边界。
        // 若 stamped_owner 显式存在但无法解析（已删除/未注册），严禁回退宿主 root，
        // 传递 None 使下游工具层严格阻断。
        let resolved_workspace_root = if let Some(root) = facts.workspace_root.clone() {
            Some(root)
        } else if let Some(owner) = stamped_owner.as_deref() {
            let sessions = self.sessions.read().unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            });
            // W1（PA-114 workspace-cwd 第二轮）：死 id 归属按归一后的 default id 解析 root
            // （与 env note 的 session_workspace_cwd 同源），不再 fail-closed；仅当 default
            // 本身也未注册时保持既有 fail-closed（None → 工具层阻断，PA-080 边界不变）。
            let normalized_owner = crate::agent::workspace::normalize_session_workspace_id(
                &sessions.list_workspaces(),
                owner,
            );
            match sessions.resolve_workspace_root(Some(&normalized_owner)) {
                Ok(root) => Some(root),
                Err(error) => {
                    eprintln!("[pony-agent] workspace root 解析失败（Fail-closed 禁止回退默认 root）：{error}");
                    None
                }
            }
        } else {
            self.workspace_root.clone()
        };
        let is_goal_round = crate::agent::goal::is_goal_round_prompt(&input.message);
        let is_direct_human = !is_goal_round;
        Some(DispatchContext {
            session_id: input.session_id.clone(),
            run_id: facts.run_id.clone(),
            turn_id: facts.turn_id.clone(),
            workspace_root: resolved_workspace_root,
            host_control_available: true,
            plan_execution_approval: input.plan_execution_approval.clone(),
            tool_authorization_mode: input.tool_authorization_mode.clone(),
            turn_cancellation: None,
            is_goal_round,
            is_direct_human,
        })
    }

    pub(crate) fn resolve_session_workspace_root(
        &self,
        session_id: Option<&str>,
    ) -> Option<PathBuf> {
        let Some(sid) = session_id else {
            return self.workspace_root.as_ref().map(PathBuf::from);
        };
        let sessions = self.sessions.read().unwrap_or_else(|e| e.into_inner());
        let owner = sessions.read_workspace_owner(sid);
        sessions
            .resolve_workspace_root(owner.as_deref())
            .ok()
            .map(PathBuf::from)
            .or_else(|| self.workspace_root.as_ref().map(PathBuf::from))
    }

    pub fn annotate_turn_trace_terminal_event(
        &self,
        session_id: Option<&str>,
        turn_id: &str,
        event_id: Option<String>,
        event_type: Option<String>,
        event_version: Option<String>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
    ) -> bool {
        let outcome = self
            .sessions
            .write()
            .unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            })
            .annotate_turn_trace_terminal_event_in_memory(
                session_id,
                turn_id,
                event_id,
                event_type,
                event_version,
                sequence,
                emitted_at_ms,
            );
        match outcome {
            Some((session_key, mutation)) => {
                self.enqueue_trace_persistence(session_key, mutation);
                true
            }
            None => false,
        }
    }

    pub fn append_turn_trace_hook_records(
        &self,
        session_id: Option<&str>,
        turn_id: &str,
        hook_trace_records: Vec<HookTraceRecord>,
    ) -> bool {
        let outcome = self
            .sessions
            .write()
            .unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            })
            .append_turn_trace_hook_records_in_memory(session_id, turn_id, hook_trace_records);
        match outcome {
            Some((session_key, mutation)) => {
                self.enqueue_trace_persistence(session_key, mutation);
                true
            }
            None => false,
        }
    }

    /// 将 trace 变更交给后台持久化队列；队列不可用时回退同步落库（尽力而为）。
    fn enqueue_trace_persistence(&self, session_id: String, mutation: SessionTraceMutation) {
        let Some(handle) = &self.trace_persistence else {
            self.sync_persist_trace(session_id, mutation);
            return;
        };
        let command = TracePersistenceCommand {
            session_id,
            mutation,
        };
        if let Err(command) = handle.try_enqueue(command) {
            self.sync_persist_trace(command.session_id, command.mutation);
        }
    }

    fn sync_persist_trace(&self, session_id: String, mutation: SessionTraceMutation) {
        self.sessions
            .write()
            .unwrap_or_else(|e| {
                tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
                e.into_inner()
            })
            .persist_trace_mutation_from_worker(&session_id, mutation);
    }

    pub fn name(&self) -> &'static str {
        "rust-core"
    }

    pub fn graph_engine(&self) -> &str {
        self.graph.name()
    }

    pub fn graph_contract_version(&self) -> &str {
        self.graph.contract_version()
    }

}

#[cfg(test)]
// 供养注记：此导入供本模块历史绑定与 tests.rs（经 use super::* 链）解析 AtomicUsize/AtomicOrdering；
// limits.rs 三个 *_override_registry 亦经 glob 链借用。清理前须先解除双消费方（reviewer-b P2-2）。
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
#[cfg(test)]
pub(crate) mod tests;
