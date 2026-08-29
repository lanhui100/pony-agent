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
use crate::agent::governed_executor::build_governed_executor;
use crate::agent::graph::{
    GraphAskResumeInjection, GraphAskWaitBinding, GraphDecision, GraphDecisionKind, GraphEngine,
    GraphRun, GraphRunStore, GraphRunner, GraphTurnHandoff,
};
use crate::agent::hooks::{
    build_observe_hook_trace_record, turn_hook_point_for_capability_mediation_hook_point,
    turn_hook_point_for_planner_hook_point, AgentHookDescriptor, AgentHookExecutor,
    AgentHookRegistry, CapabilityMediationEnvelope, CapabilityMediationHookPoint,
    HookFailurePolicy, HookPatchConflictPolicy, HookPatchOperation, HookPatchOperationKind,
    HookStructuredResult, HookTraceRecord, NoopHookExecutor, PlannerFactsEnvelope,
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
    HistoryBranch, HistoryCheckoutMode, HistoryCursor, HistoryNode, SessionAttachment,
    SessionOverview, SessionSnapshot, SessionStore, TraceTimelineEntry, TurnHistoryMessage,
    TurnTraceRecord,
};
use crate::agent::telemetry::{
    DefaultTurnTelemetryBuilder, ProviderCallCacheRecord, ProviderLatencyKind, ProviderRequestKind,
    TurnTelemetryBuilder, TurnToolActivity, TurnTraceStep,
};
use std::path::PathBuf;
use crate::agent::tools::{
    builtin_tools, canonical_tool_name, default_permission_facts_for_name, tool_error_from_output,
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
    emit_stream_failed, emit_turn_failed, normalize_user_message, preview_text, provider_decision,
    provider_decision_stream, provider_event_meta, provider_failure_message, provider_followup,
    provider_followup_stream, runtime_log, stream_reasoning_chunks, stream_text_chunks,
    token_usage_parts, ModelHopTraceContent, PersistedTurnOutcome, PlannedTurn, PreparedTurn,
    ProviderEventMeta, SyncToolTurnOutcome, TurnEventEnvelope, TurnEventSink,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::Instant;

pub mod turn_runner;

mod ingress_mediation;
mod session_ops;
mod tool_exec;
mod turn_persist;
mod turn_prep;

mod blocked_records;
mod builder;
mod limits;
mod planner_patches;
mod stream_support;
mod tool_recovery;
mod trace_timeline;
mod turn_control;
mod turn_sync;
mod turn_stream;
mod types;

pub use builder::{AgentRuntimeBuilder, DesktopRuntimePreset};
pub use types::{PlannerGraphDecisionDispatchOutcome, RunTurnFacts, SUSPENDED_TURN_PHASE, TurnInput, TurnResult, TurnStreamEvent};

use blocked_records::*;
use limits::*;
use planner_patches::*;
use stream_support::*;
use tool_recovery::*;
use trace_timeline::*;


// 原 pub(crate) 项的 crate 级寻址契约保持（reviewer-b P2-1 后果二修复）
pub(crate) use planner_patches::{apply_capability_argument_patches, apply_planner_patches, normalized_arguments_from_summary};
pub(crate) use types::{CapabilityMediationDispatchOutcome, GraphDecisionDispatchOutcome, HookDispatchOutcome, PlannerDispatchOutcome};
use types::*;

pub struct AgentRuntime {
    graph: GraphEngine,
    sessions: Arc<RwLock<SessionStore>>,
    provider_resolver: Box<dyn ProviderSelectionResolver>,
    capability_registry: CapabilityRegistry,
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
    /// The governed dispatcher the default tool executor executes through, cached once so the
    /// runtime and the host control plane share the exact same `Arc` (design.md Decision 5,
    /// P1-1). Computed from the tool executor at first use; `None` when the executor is not the
    /// governed adapter.
    governed_dispatcher: OnceLock<Option<Arc<GovernedDispatcher>>>,
}

impl AgentRuntime {
    pub fn new() -> Self {
        DesktopRuntimePreset::build()
    }

    /// 当前 workspace root（PA-078 宿主 `get_workspace_root` 读取）。
    pub fn workspace_root(&self) -> Option<&str> {
        self.workspace_root.as_deref()
    }

    pub fn with_dependencies(
        sessions: SessionStore,
        provider_resolver: Box<dyn ProviderSelectionResolver>,
        tool_executor: Box<dyn ToolExecutor>,
        planner: Box<dyn TurnPlanner>,
        context_builder: Box<dyn TurnContextBuilder>,
        telemetry_builder: Box<dyn TurnTelemetryBuilder>,
    ) -> Self {
        let persisted_mcp_snapshots = sessions.list_persisted_mcp_source_snapshots();
        let persisted_skill_snapshots = sessions.list_persisted_skill_source_snapshots();
        let mut capability_registry = CapabilityRegistry::new();
        for snapshot in persisted_mcp_snapshots {
            capability_registry.replace_mcp_source_snapshot(snapshot);
        }
        for snapshot in persisted_skill_snapshots {
            let _ = capability_registry.replace_skill_source_snapshot(snapshot);
        }
        let sessions_arc = Arc::new(RwLock::new(sessions));
        let trace_persistence = Some(spawn_trace_persistence_worker(Arc::clone(&sessions_arc)));
        Self {
            graph: GraphEngine::new("state-machine-v1"),
            sessions: sessions_arc,
            provider_resolver,
            capability_registry,
            hook_registry: AgentHookRegistry::new(),
            hook_executor: Box::new(NoopHookExecutor),
            tool_executor,
            planner,
            context_builder,
            telemetry_builder,
            trace_persistence,
            graph_runs: None,
            workspace_root: None,
            governed_dispatcher: OnceLock::new(),
        }
    }

    pub fn sessions_handle(&self) -> Arc<RwLock<SessionStore>> {
        Arc::clone(&self.sessions)
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

    /// Consume the one-shot Ask resume injection for a run from the shared graph run store, if
    /// one is pending (design.md Decision 5, phase-4 P0). The host answers through
    /// `graph_resume_ask` (which persists the injection), and the next graph-run turn consumes it
    /// exactly once here — before the provider request is built — so the terminal tool result is
    /// seeded into the turn's provider context.
    fn take_pending_ask_injection(&self, run_id: Option<&str>) -> Option<GraphAskResumeInjection> {
        let run_id = run_id?;
        let store_arc = self.graph_run_store()?;
        let mut store = store_arc.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] graph run store poisoned: {e}, recovering");
            e.into_inner()
        });
        store.take_ask_injection(run_id)
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

    /// Apply the real session/run/turn/workspace facts to the governed executor before a turn
    /// executes, so a persisted `PendingControlRequest` (Ask) is bound to the real invocation
    /// (design.md Decision 5, P1-1 wiring / P2-8). `None` run/turn facts (plain `run_turn`)
    /// still bind the session; `host_control_available` stays `true` so an interactive host can
    /// answer. When the executor is not governed this is a no-op.
    pub(crate) fn apply_governed_turn_context(&self, input: &TurnInput, facts: &RunTurnFacts) {
        // 三级树 #3（确定性根解析）：先按 input 盖章/归一（未注册 id 由 stamp
        // 归一为 default），随后一律以"盖章后的会话权威归属"解析工具根——
        // 前端 id 漂移/旧列表快照对后端免疫，杜绝"会话在新工作区、工具在默认 root"。
        let stamped_owner: Option<String> = 'stamp: {
            let Some(session_id) = input.session_id.as_deref() else {
                break 'stamp None;
            };
            let mut sessions = self.sessions.write().unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
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
        let Some(governed) = self
            .tool_executor
            .as_any()
            .and_then(|any| any.downcast_ref::<GovernedToolExecutor>())
        else {
            return;
        };
        // PA-096 Phase 1：会话级 workspace root 解析——严格 Fail-closed 边界。
        // 若 stamped_owner 显式存在但无法解析（已删除/未注册），严禁回退宿主 root，
        // 传递 None 使下游工具层严格阻断。
        let resolved_workspace_root = if let Some(root) = facts.workspace_root.clone() {
            Some(root)
        } else if let Some(owner) = stamped_owner.as_deref() {
            let sessions = self.sessions.read().unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            });
            match sessions.resolve_workspace_root(Some(owner)) {
                Ok(root) => Some(root),
                Err(error) => {
                    eprintln!("[pony-agent] workspace root 解析失败（Fail-closed 禁止回退默认 root）：{error}");
                    None
                }
            }
        } else {
            self.workspace_root.clone()
        };
        governed.set_context(DispatchContext {
            session_id: input.session_id.clone(),
            run_id: facts.run_id.clone(),
            turn_id: facts.turn_id.clone(),
            workspace_root: resolved_workspace_root,
            host_control_available: true,
        });
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
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
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
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
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
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
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
mod tests;
