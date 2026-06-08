use crate::agent::capability_bridge::{
    enrich_mcp_source_snapshot, enrich_skill_source_snapshot, CapabilityFailureKind,
    CapabilityRegistry, CapabilityToolExecutionResult, McpSourceSnapshot, SkillFailureLayer,
    SkillInvocationRequest, SkillSourceSnapshot, SkillToolExecutionResult,
};
use crate::agent::config::{
    ProviderReasoningEffort, ProviderRegistryStore, ProviderSelectionResolver,
};
use crate::agent::context::{DefaultTurnContextBuilder, RetrievedContextState, TurnContextBuilder};
use crate::agent::execution_control::ExecutionCheckpoint;
use crate::agent::execution_control::ExecutionControlRegistry;
use crate::agent::graph::{
    GraphDecision, GraphDecisionKind, GraphEngine, GraphRun, GraphTurnHandoff,
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
    provider_native_assistant_tool_call_message_with_reasoning_value,
    provider_native_tool_result_message, provider_native_user_message, BuildContextObservation,
    ProviderDecision, ProviderManager, ProviderRequest, ProviderResponse, ProviderStreamChunk,
    TokenUsage,
};
use crate::agent::session::{
    HistoryBranch, HistoryCheckoutMode, HistoryCursor, HistoryNode, SessionAttachment,
    SessionOverview, SessionSnapshot, SessionStore, TraceTimelineEntry, TurnHistoryMessage,
    TurnTraceRecord,
};
use crate::agent::telemetry::{
    DefaultTurnTelemetryBuilder, ProviderCallCacheRecord, ProviderLatencyKind, ProviderRequestKind,
    TurnTelemetryBuilder, TurnToolActivity, TurnTraceStep,
};
use crate::agent::tools::{builtin_tools, ToolCall, ToolDefinition, ToolExecutor, ToolRouter};
use crate::agent::turn_flow::{
    build_failed_turn_result, build_failed_turn_result_with_hooks,
    build_terminal_turn_event_envelope, emit_stream_cancelled, emit_stream_event,
    emit_stream_failed, emit_turn_failed, normalize_user_message, preview_text, provider_decision,
    provider_decision_stream, provider_event_meta, provider_failure_message, provider_followup,
    provider_followup_stream, runtime_log, stream_reasoning_chunks, stream_text_chunks,
    token_usage_parts, PersistedTurnOutcome, PlannedTurn, PreparedTurn, ProviderEventMeta,
    SyncToolTurnOutcome, TurnEventEnvelope, TurnEventSink,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::cell::{Cell, RefCell};
use std::collections::BTreeSet;
use std::path::Path;
use std::rc::Rc;
#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::OnceLock;
use std::time::Instant;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnInput {
    pub message: String,
    pub display_message: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<ProviderReasoningEffort>,
    pub session_id: Option<String>,
    pub node_id: Option<String>,
    #[serde(default)]
    pub history: Vec<TurnHistoryMessage>,
    #[serde(default)]
    pub images: Vec<TurnInputImage>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnResult {
    pub event_id: Option<String>,
    pub event_type: Option<String>,
    pub event_version: Option<String>,
    pub sequence: Option<u64>,
    pub emitted_at_ms: Option<u64>,
    pub phase: String,
    pub provider_requested_name: String,
    pub provider_name: String,
    pub provider_protocol: String,
    pub provider_model: String,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub build_context_observation: Option<BuildContextObservation>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    pub user_message: String,
    pub assistant_message: String,
    pub trace_steps: Vec<TurnTraceStep>,
    pub trace_timeline: Vec<TraceTimelineEntry>,
    pub tool_activities: Vec<TurnToolActivity>,
    pub provider_call_records: Vec<ProviderCallCacheRecord>,
    pub hook_trace_records: Vec<HookTraceRecord>,
    pub session_summary: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnStreamEvent {
    pub event_id: Option<String>,
    pub session_id: Option<String>,
    pub turn_id: String,
    pub kind: String,
    pub event_type: Option<String>,
    pub event_version: Option<String>,
    pub sequence: Option<u64>,
    pub emitted_at_ms: Option<u64>,
    pub phase: Option<String>,
    pub text: Option<String>,
    pub reasoning_content: Option<String>,
    pub error: Option<String>,
    pub provider_requested_name: Option<String>,
    pub provider_name: Option<String>,
    pub provider_protocol: Option<String>,
    pub provider_model: Option<String>,
    pub provider_source: Option<String>,
    pub provider_mode: Option<String>,
    pub fallback_reason: Option<String>,
    pub build_context_observation: Option<BuildContextObservation>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    pub trace_steps: Option<Vec<TurnTraceStep>>,
    pub trace_timeline: Option<Vec<TraceTimelineEntry>>,
    pub tool_activities: Option<Vec<TurnToolActivity>>,
    pub provider_call_records: Option<Vec<ProviderCallCacheRecord>>,
    pub hook_trace_records: Option<Vec<HookTraceRecord>>,
    pub session_summary: Option<String>,
}

const DEFAULT_MAX_TOOL_HOPS_PER_TURN: usize = 1024;
const MAX_ALLOWED_TOOL_HOPS_PER_TURN: usize = 4096;
const MAX_TOOL_HOPS_ENV: &str = "PONY_AGENT_MAX_TOOL_HOPS_PER_TURN";
const DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN: usize = 4;
const MAX_ALLOWED_TOOL_FOLLOWUPS_PER_TURN: usize = 32;
const MAX_TOOL_FOLLOWUPS_ENV: &str = "PONY_AGENT_MAX_TOOL_FOLLOWUPS_PER_TURN";
const STREAM_REASONING_BATCH_CHARS: usize = 96;
const MAX_TURN_IMAGES: usize = 3;
const MAX_TURN_IMAGE_BYTES: u64 = 24 * 1024 * 1024;
const CANCELLED_TURN_MESSAGE: &str = "This turn was cancelled.";
const HOOK_FAILTURN_HANDLED_SENTINEL: &str = "__hook_failturn_handled__";

#[derive(Clone)]
struct ToolTurnHopRecord {
    assistant_output_text: String,
    assistant_reasoning_content: Option<String>,
    assistant_reasoning_content_value: Option<Value>,
    tool_call: ToolCall,
    tool_result: crate::agent::tools::ToolResult,
}

struct RecoveredToolFollowup {
    response: ProviderResponse,
    provider_call_record: ProviderCallCacheRecord,
}

struct HookDispatchOutcome {
    trace_records: Vec<HookTraceRecord>,
    fail_turn_error: Option<String>,
}

struct CapabilityMediationDispatchOutcome {
    arguments: Value,
    trace_records: Vec<HookTraceRecord>,
    blocked_error: Option<String>,
    fail_turn_error: Option<String>,
}

struct PlannerDispatchOutcome {
    decision: Option<ProviderDecision>,
    selected_tool_call: Option<ToolCall>,
    trace_records: Vec<HookTraceRecord>,
    blocked_error: Option<String>,
    fail_turn_error: Option<String>,
}

pub struct PlannerGraphDecisionDispatchOutcome {
    pub decision: GraphDecision,
    pub trace_records: Vec<HookTraceRecord>,
}

struct GraphDecisionDispatchOutcome {
    decision: GraphDecision,
    trace_records: Vec<HookTraceRecord>,
    blocked_error: Option<String>,
    fail_turn_error: Option<String>,
}

struct NormalizedToolDirective {
    tool_call: ToolCall,
    assistant_message: Option<Value>,
}

pub struct AgentRuntime {
    graph: GraphEngine,
    sessions: SessionStore,
    provider_resolver: Box<dyn ProviderSelectionResolver>,
    capability_registry: CapabilityRegistry,
    hook_registry: AgentHookRegistry,
    hook_executor: Box<dyn AgentHookExecutor>,
    tool_executor: Box<dyn ToolExecutor>,
    planner: Box<dyn TurnPlanner>,
    context_builder: Box<dyn TurnContextBuilder>,
    telemetry_builder: Box<dyn TurnTelemetryBuilder>,
}

impl AgentRuntime {
    pub fn new() -> Self {
        Self::with_dependencies(
            SessionStore::new(),
            Box::new(ProviderRegistryStore::new()),
            Box::new(ToolRouter::new()),
            Box::new(LocalTurnPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        )
    }

    pub(crate) fn with_dependencies(
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
        Self {
            graph: GraphEngine::new("state-machine-v1"),
            sessions,
            provider_resolver,
            capability_registry,
            hook_registry: AgentHookRegistry::new(),
            hook_executor: Box::new(NoopHookExecutor),
            tool_executor,
            planner,
            context_builder,
            telemetry_builder,
        }
    }

    pub fn annotate_turn_trace_terminal_event(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        event_id: Option<String>,
        event_type: Option<String>,
        event_version: Option<String>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
    ) -> bool {
        self.sessions
            .annotate_turn_trace_terminal_event(
                session_id,
                turn_id,
                event_id,
                event_type,
                event_version,
                sequence,
                emitted_at_ms,
            )
            .is_some()
    }

    pub fn append_turn_trace_hook_records(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        hook_trace_records: Vec<HookTraceRecord>,
    ) -> bool {
        self.sessions
            .append_turn_trace_hook_records(session_id, turn_id, hook_trace_records)
            .is_some()
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

    pub fn apply_mcp_source_snapshot(&mut self, snapshot: McpSourceSnapshot) {
        let snapshot = enrich_mcp_source_snapshot(snapshot);
        self.sessions.persist_mcp_source_snapshot(snapshot.clone());
        self.capability_registry
            .replace_mcp_source_snapshot(snapshot);
    }

    pub fn dispatch_mcp_source_ingress_hooks(
        &self,
        snapshot: &McpSourceSnapshot,
    ) -> Result<Vec<HookTraceRecord>, String> {
        let mut dispatch = self.dispatch_capability_mediation_hooks(
            CapabilityMediationHookPoint::McpSourceIngress,
            &self.build_mcp_source_ingress_envelope(snapshot),
        );
        if let Some(error) = dispatch.fail_turn_error.take() {
            return Err(error);
        }
        if let Some(error) = dispatch.blocked_error.take() {
            return Err(error);
        }
        Ok(dispatch.trace_records)
    }

    pub fn apply_skill_source_snapshot(
        &mut self,
        snapshot: SkillSourceSnapshot,
    ) -> Result<(), String> {
        let snapshot = enrich_skill_source_snapshot(snapshot);
        self.sessions
            .persist_skill_source_snapshot(snapshot.clone());
        self.capability_registry
            .replace_skill_source_snapshot(snapshot)
    }

    pub fn dispatch_skill_source_ingress_hooks(
        &self,
        snapshot: &SkillSourceSnapshot,
    ) -> Result<Vec<HookTraceRecord>, String> {
        let mut dispatch = self.dispatch_capability_mediation_hooks(
            CapabilityMediationHookPoint::SkillSourceIngress,
            &self.build_skill_source_ingress_envelope(snapshot),
        );
        if let Some(error) = dispatch.fail_turn_error.take() {
            return Err(error);
        }
        if let Some(error) = dispatch.blocked_error.take() {
            return Err(error);
        }
        Ok(dispatch.trace_records)
    }

    pub fn capability_registry_snapshot(&self) -> CapabilityRegistry {
        self.capability_registry.clone()
    }

    pub fn register_hook_descriptor(
        &mut self,
        descriptor: AgentHookDescriptor,
    ) -> Result<(), String> {
        self.hook_registry.register(descriptor)
    }

    #[cfg(test)]
    pub fn set_hook_executor_for_test(&mut self, hook_executor: Box<dyn AgentHookExecutor>) {
        self.hook_executor = hook_executor;
    }

    #[cfg(test)]
    pub fn set_history_state_hook_executor_for_test(
        &mut self,
        hook_executor: Box<dyn crate::agent::hooks::HistoryStateHookExecutor>,
    ) {
        self.sessions
            .set_history_state_hook_executor_for_test(hook_executor);
    }

    #[cfg(test)]
    pub fn record_turn_trace_for_test(&mut self, session_id: Option<&str>, trace: TurnTraceRecord) {
        self.sessions.record_turn_trace(session_id, trace);
    }

    fn dispatch_hook_trace_records(&self, hook_point: TurnHookPoint) -> HookDispatchOutcome {
        let descriptors = self.hook_registry.list_for_hook_point(&hook_point);
        let mut records = Vec::with_capacity(descriptors.len());
        let mut fail_turn_error = None;

        for (index, descriptor) in descriptors.into_iter().enumerate() {
            match self.hook_executor.execute(descriptor, hook_point.clone()) {
                Ok(mut result) => {
                    result.hook_order = (index + 1) as u32;
                    records.push(result.to_trace_record());
                }
                Err(error) => {
                    let (result_kind, structured_result) =
                        crate::agent::hooks::normalized_result_for_class(&descriptor.class);
                    records.push(HookTraceRecord {
                        hook_name: descriptor.name.clone(),
                        hook_class: descriptor.class.clone(),
                        hook_point: hook_point.clone(),
                        hook_order: (index + 1) as u32,
                        result_kind,
                        structured_result,
                        blocked: matches!(
                            descriptor.default_failure_policy,
                            HookFailurePolicy::FailTurn
                        ),
                        elapsed_ms: 0,
                        input_summary: Some(format!("hook executor failed: {error}")),
                        persistence_evidence_ref: None,
                        summary: format!(
                            "hook execution failed under {:?}: {error}",
                            descriptor.default_failure_policy
                        ),
                    });
                    if matches!(
                        descriptor.default_failure_policy,
                        HookFailurePolicy::FailTurn
                    ) {
                        fail_turn_error = Some(format!(
                            "hook `{}` forced turn failure at `{:?}`: {error}",
                            descriptor.name, hook_point
                        ));
                        break;
                    }
                }
            }
        }

        HookDispatchOutcome {
            trace_records: records,
            fail_turn_error,
        }
    }

    fn dispatch_capability_mediation_hooks(
        &self,
        hook_point: CapabilityMediationHookPoint,
        envelope: &CapabilityMediationEnvelope,
    ) -> CapabilityMediationDispatchOutcome {
        let turn_hook_point = turn_hook_point_for_capability_mediation_hook_point(&hook_point);
        let descriptors = self.hook_registry.list_for_hook_point(&turn_hook_point);
        let mut records = Vec::with_capacity(descriptors.len());
        let mut execution_results = Vec::new();
        let mut fail_turn_error = None;
        let mut blocked_error = None;

        for (index, descriptor) in descriptors.into_iter().enumerate() {
            match self.hook_executor.execute_capability_mediation(
                descriptor,
                hook_point.clone(),
                envelope,
            ) {
                Ok(mut result) => {
                    result.hook_order = (index + 1) as u32;
                    if let HookStructuredResult::Deny(deny) = &result.structured_result {
                        blocked_error = Some(format!(
                            "hook `{}` blocked capability mediation: {}",
                            descriptor.name, deny.message
                        ));
                    }
                    records.push(result.to_trace_record());
                    execution_results.push(result);
                    if blocked_error.is_some() {
                        break;
                    }
                }
                Err(error) => {
                    let (result_kind, structured_result) =
                        crate::agent::hooks::normalized_result_for_class(&descriptor.class);
                    records.push(HookTraceRecord {
                        hook_name: descriptor.name.clone(),
                        hook_class: descriptor.class.clone(),
                        hook_point: turn_hook_point.clone(),
                        hook_order: (index + 1) as u32,
                        result_kind,
                        structured_result,
                        blocked: matches!(
                            descriptor.default_failure_policy,
                            HookFailurePolicy::FailTurn
                        ),
                        elapsed_ms: 0,
                        input_summary: Some(format!("hook executor failed: {error}")),
                        persistence_evidence_ref: None,
                        summary: format!(
                            "hook execution failed under {:?}: {error}",
                            descriptor.default_failure_policy
                        ),
                    });
                    if matches!(
                        descriptor.default_failure_policy,
                        HookFailurePolicy::FailTurn
                    ) {
                        fail_turn_error = Some(format!(
                            "hook `{}` forced turn failure at `{:?}`: {error}",
                            descriptor.name, turn_hook_point
                        ));
                        break;
                    }
                }
            }
        }

        let arguments = if fail_turn_error.is_none() && blocked_error.is_none() {
            apply_capability_argument_patches(
                &hook_point,
                &envelope.argument_summary,
                &execution_results,
            )
            .unwrap_or_else(|error| {
                fail_turn_error = Some(error);
                normalized_arguments_from_summary(&envelope.argument_summary)
            })
        } else {
            normalized_arguments_from_summary(&envelope.argument_summary)
        };

        CapabilityMediationDispatchOutcome {
            arguments,
            trace_records: records,
            blocked_error,
            fail_turn_error,
        }
    }

    fn dispatch_planner_hooks(
        &self,
        hook_point: PlannerHookPoint,
        envelope: &PlannerFactsEnvelope,
        decision: Option<ProviderDecision>,
        selected_tool_call: Option<ToolCall>,
    ) -> PlannerDispatchOutcome {
        let turn_hook_point = turn_hook_point_for_planner_hook_point(&hook_point);
        let descriptors = self.hook_registry.list_for_hook_point(&turn_hook_point);
        let mut records = Vec::with_capacity(descriptors.len());
        let mut execution_results = Vec::new();
        let mut fail_turn_error = None;
        let mut blocked_error = None;

        for (index, descriptor) in descriptors.into_iter().enumerate() {
            match self
                .hook_executor
                .execute_planner(descriptor, hook_point.clone(), envelope)
            {
                Ok(mut result) => {
                    result.hook_order = (index + 1) as u32;
                    if let HookStructuredResult::Deny(deny) = &result.structured_result {
                        blocked_error = Some(format!(
                            "hook `{}` blocked planner mediation: {}",
                            descriptor.name, deny.message
                        ));
                    }
                    records.push(result.to_trace_record());
                    execution_results.push(result);
                    if blocked_error.is_some() {
                        break;
                    }
                }
                Err(error) => {
                    let (result_kind, structured_result) =
                        crate::agent::hooks::normalized_result_for_class(&descriptor.class);
                    records.push(HookTraceRecord {
                        hook_name: descriptor.name.clone(),
                        hook_class: descriptor.class.clone(),
                        hook_point: turn_hook_point.clone(),
                        hook_order: (index + 1) as u32,
                        result_kind,
                        structured_result,
                        blocked: matches!(
                            descriptor.default_failure_policy,
                            HookFailurePolicy::FailTurn
                        ),
                        elapsed_ms: 0,
                        input_summary: Some(format!("hook executor failed: {error}")),
                        persistence_evidence_ref: None,
                        summary: format!(
                            "hook execution failed under {:?}: {error}",
                            descriptor.default_failure_policy
                        ),
                    });
                    if matches!(
                        descriptor.default_failure_policy,
                        HookFailurePolicy::FailTurn
                    ) {
                        fail_turn_error = Some(format!(
                            "hook `{}` forced turn failure at `{:?}`: {error}",
                            descriptor.name, turn_hook_point
                        ));
                        break;
                    }
                }
            }
        }

        let (decision, selected_tool_call) = if fail_turn_error.is_none() && blocked_error.is_none()
        {
            apply_planner_patches(
                &hook_point,
                decision,
                selected_tool_call,
                &execution_results,
            )
            .unwrap_or_else(|error| {
                fail_turn_error = Some(error);
                (None, None)
            })
        } else {
            (decision, selected_tool_call)
        };

        PlannerDispatchOutcome {
            decision,
            selected_tool_call,
            trace_records: records,
            blocked_error,
            fail_turn_error,
        }
    }

    fn dispatch_graph_decision_hooks(
        &self,
        envelope: &PlannerFactsEnvelope,
        mut decision: GraphDecision,
    ) -> GraphDecisionDispatchOutcome {
        let hook_point = PlannerHookPoint::GraphDecision;
        let turn_hook_point = turn_hook_point_for_planner_hook_point(&hook_point);
        let descriptors = self.hook_registry.list_for_hook_point(&turn_hook_point);
        let mut records = Vec::with_capacity(descriptors.len());
        let mut execution_results = Vec::new();
        let mut fail_turn_error = None;
        let mut blocked_error = None;

        for (index, descriptor) in descriptors.into_iter().enumerate() {
            match self
                .hook_executor
                .execute_planner(descriptor, hook_point.clone(), envelope)
            {
                Ok(mut result) => {
                    result.hook_order = (index + 1) as u32;
                    if let HookStructuredResult::Deny(deny) = &result.structured_result {
                        blocked_error = Some(format!(
                            "hook `{}` blocked planner graph decision: {}",
                            descriptor.name, deny.message
                        ));
                    }
                    records.push(result.to_trace_record());
                    execution_results.push(result);
                    if blocked_error.is_some() {
                        break;
                    }
                }
                Err(error) => {
                    let (result_kind, structured_result) =
                        crate::agent::hooks::normalized_result_for_class(&descriptor.class);
                    records.push(HookTraceRecord {
                        hook_name: descriptor.name.clone(),
                        hook_class: descriptor.class.clone(),
                        hook_point: turn_hook_point.clone(),
                        hook_order: (index + 1) as u32,
                        result_kind,
                        structured_result,
                        blocked: matches!(
                            descriptor.default_failure_policy,
                            HookFailurePolicy::FailTurn
                        ),
                        elapsed_ms: 0,
                        input_summary: Some(format!("hook executor failed: {error}")),
                        persistence_evidence_ref: None,
                        summary: format!(
                            "hook execution failed under {:?}: {error}",
                            descriptor.default_failure_policy
                        ),
                    });
                    if matches!(
                        descriptor.default_failure_policy,
                        HookFailurePolicy::FailTurn
                    ) {
                        fail_turn_error = Some(format!(
                            "hook `{}` forced turn failure at `{:?}`: {error}",
                            descriptor.name, turn_hook_point
                        ));
                        break;
                    }
                }
            }
        }

        if fail_turn_error.is_none() && blocked_error.is_none() {
            if let Err(error) = apply_graph_decision_patches(&mut decision, &execution_results) {
                fail_turn_error = Some(error);
            }
        }

        GraphDecisionDispatchOutcome {
            decision,
            trace_records: records,
            blocked_error,
            fail_turn_error,
        }
    }

    #[cfg(test)]
    pub fn inspect_capability(
        &self,
        capability_id: &str,
    ) -> Option<crate::agent::capability_bridge::CapabilityView> {
        self.capability_registry.inspect_capability(capability_id)
    }

    #[cfg(test)]
    pub fn inspect_capability_source(
        &self,
        source_id: &str,
    ) -> Option<crate::agent::capability_bridge::CapabilitySourceView> {
        self.capability_registry.inspect_source(source_id)
    }

    #[cfg(test)]
    pub fn inspect_skill(
        &self,
        skill_id: &str,
    ) -> Option<crate::agent::capability_bridge::SkillDescriptor> {
        self.capability_registry.inspect_skill(skill_id)
    }

    #[cfg(test)]
    pub fn inspect_skill_source(
        &self,
        source_id: &str,
    ) -> Option<crate::agent::capability_bridge::SkillSourceView> {
        self.capability_registry.inspect_skill_source(source_id)
    }

    #[cfg(test)]
    pub fn register_mcp_capability_for_test(
        &mut self,
        capability: crate::agent::capability_bridge::CapabilityView,
    ) {
        self.capability_registry.register_mcp_capability(capability);
    }

    #[cfg(test)]
    pub fn remove_mcp_source_for_test(&mut self, source_id: &str) {
        self.capability_registry.remove_source_for_test(source_id);
    }

    #[allow(dead_code)]
    pub fn start_graph_run(
        &self,
        run_id: impl Into<String>,
        goal: impl Into<String>,
        session_id: Option<&str>,
    ) -> GraphRun {
        self.graph.start_run(run_id, goal, session_id)
    }

    pub fn list_sessions(&self) -> Vec<SessionOverview> {
        self.sessions.list_sessions()
    }

    pub fn load_session_snapshot(&mut self, session_id: Option<&str>) -> SessionSnapshot {
        self.load_session_snapshot_at(session_id, None)
    }

    pub fn load_session_snapshot_at(
        &mut self,
        session_id: Option<&str>,
        node_id: Option<&str>,
    ) -> SessionSnapshot {
        self.sessions.snapshot_at(session_id, node_id, &[])
    }

    pub fn inspect_retrieved_context(
        &mut self,
        session_id: Option<&str>,
        run: Option<&GraphRun>,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> RetrievedContextState {
        self.inspect_retrieved_context_at(session_id, None, run, checkpoint)
    }

    pub fn inspect_retrieved_context_at(
        &mut self,
        session_id: Option<&str>,
        node_id: Option<&str>,
        run: Option<&GraphRun>,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> RetrievedContextState {
        let snapshot = self.load_session_snapshot_at(session_id, node_id);
        let inspection_user_message = snapshot
            .history
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| message.content.as_str())
            .unwrap_or("");
        self.context_builder.retrieve_context_state(
            inspection_user_message,
            &[],
            &snapshot,
            run,
            checkpoint,
        )
    }

    #[allow(dead_code)]
    pub fn build_graph_turn_handoff(
        &mut self,
        run: Option<&GraphRun>,
        turn_id: Option<&str>,
        session_id: Option<&str>,
        result: &TurnResult,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> GraphTurnHandoff {
        let snapshot = self.load_session_snapshot(session_id);
        let retrieved = self.context_builder.retrieve_context_state(
            &result.user_message,
            &[],
            &snapshot,
            run,
            checkpoint,
        );
        self.graph
            .build_turn_handoff(turn_id, session_id, result, &retrieved)
    }

    #[allow(dead_code)]
    pub fn decide_graph_after_turn(
        &mut self,
        turn_id: Option<&str>,
        session_id: Option<&str>,
        result: &TurnResult,
        checkpoint: Option<&ExecutionCheckpoint>,
    ) -> GraphDecision {
        let handoff = self.build_graph_turn_handoff(None, turn_id, session_id, result, checkpoint);
        self.graph.decide_after_turn(&handoff)
    }

    #[allow(dead_code)]
    pub fn decide_graph_after_turn_with_planner(
        &mut self,
        run: &GraphRun,
        turn_id: Option<&str>,
        session_id: Option<&str>,
        result: &TurnResult,
        checkpoint: Option<&ExecutionCheckpoint>,
        planner: &dyn GraphPlanner,
    ) -> Result<PlannerGraphDecisionDispatchOutcome, String> {
        let handoff =
            self.build_graph_turn_handoff(Some(run), turn_id, session_id, result, checkpoint);
        let decision = self
            .graph
            .decide_after_turn_with_planner(run, &handoff, planner);
        let mut dispatch = self.dispatch_graph_decision_hooks(
            &self.build_planner_graph_decision_envelope(run, &decision),
            decision,
        );
        if let Some(error) = dispatch.fail_turn_error.take() {
            return Err(error);
        }
        if let Some(error) = dispatch.blocked_error.take() {
            return Err(error);
        }
        Ok(PlannerGraphDecisionDispatchOutcome {
            decision: dispatch.decision,
            trace_records: dispatch.trace_records,
        })
    }

    pub fn remove_session(&mut self, session_id: &str) -> Vec<SessionOverview> {
        self.sessions.remove_session(session_id)
    }

    pub fn load_history_graph(
        &mut self,
        session_id: Option<&str>,
    ) -> (Vec<HistoryNode>, Vec<HistoryBranch>, HistoryCursor) {
        self.sessions.load_history_graph(session_id)
    }

    pub fn load_history_cursor(&mut self, session_id: Option<&str>) -> HistoryCursor {
        self.sessions.load_history_cursor(session_id)
    }

    pub fn checkout_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
        mode: HistoryCheckoutMode,
    ) -> Result<SessionSnapshot, String> {
        self.sessions
            .checkout_history_node(session_id, node_id, mode)
    }

    pub fn restore_branch_head(
        &mut self,
        session_id: Option<&str>,
        branch_id: Option<&str>,
    ) -> Result<SessionSnapshot, String> {
        self.sessions.restore_branch_head(session_id, branch_id)
    }

    pub fn fork_from_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
    ) -> Result<SessionSnapshot, String> {
        self.sessions.fork_from_history_node(session_id, node_id)
    }

    pub fn switch_history_branch(
        &mut self,
        session_id: Option<&str>,
        branch_id: &str,
    ) -> Result<SessionSnapshot, String> {
        self.sessions.switch_history_branch(session_id, branch_id)
    }

    fn prepare_turn(
        &mut self,
        input: &TurnInput,
        reject_empty: bool,
    ) -> Result<PreparedTurn, String> {
        let user_message = if reject_empty {
            let trimmed = input.message.trim();
            if trimmed.is_empty() {
                return Err("Message is empty.".to_string());
            }
            trimmed.to_string()
        } else {
            normalize_user_message(&input.message)
        };

        let session = self.sessions.snapshot_at(
            input.session_id.as_deref(),
            input.node_id.as_deref(),
            &input.history,
        );
        let provider = self.resolve_provider(input);
        let preliminary_retrieved =
            self.context_builder
                .retrieve_context_state(&user_message, &[], &session, None, None);
        let effective_images =
            self.resolve_turn_images(input, &preliminary_retrieved, &provider)?;
        let tools = builtin_tools();
        let planner_skills = self.capability_registry.list_skills_for_planner();
        let retrieved = if effective_images.is_empty() {
            preliminary_retrieved
        } else {
            self.context_builder.retrieve_context_state(
                &user_message,
                &effective_images,
                &session,
                None,
                None,
            )
        };
        let planning_request = self.context_builder.build_request(
            self.graph.name(),
            &provider,
            &retrieved,
            &planner_skills,
        );
        let build_context_observation = build_context_observation(&planning_request, &tools);
        let display_message = input
            .display_message
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| user_message.clone());

        Ok(PreparedTurn {
            user_message,
            display_message,
            retrieved,
            provider,
            tools,
            planner_skills,
            planning_request,
            build_context_observation,
        })
    }

    fn plan_turn(&self, prepared: &PreparedTurn) -> Result<PlannedTurn, String> {
        let preflight_decision = self.planner.preflight_decision(
            &prepared.user_message,
            prepared.retrieved.planner_history(),
            &prepared.planner_skills,
        );
        let mut preflight_dispatch = self.dispatch_planner_hooks(
            PlannerHookPoint::TurnPreflight,
            &self.build_planner_preflight_envelope(
                &prepared.user_message,
                prepared.retrieved.planner_history(),
                &prepared.planner_skills,
                preflight_decision.as_ref(),
            ),
            preflight_decision,
            None,
        );
        if let Some(error) = preflight_dispatch.fail_turn_error.take() {
            return Err(error);
        }
        if let Some(error) = preflight_dispatch.blocked_error.take() {
            return Err(error);
        }
        let preflight_decision = preflight_dispatch.decision;
        let mut planner_hook_trace_records = preflight_dispatch.trace_records;
        planner_hook_trace_records.push(self.build_planner_preflight_trace_record(
            &prepared.user_message,
            &prepared.planner_skills,
            preflight_decision.as_ref(),
        ));

        let (mut first_decision, initial_decision_duration_ms) =
            if prepared.provider.requires_provider_native_tool_flow() {
                match preflight_decision {
                    Some(decision) if planner_decision_can_override_native_tool_flow(&decision) => {
                        (decision, None)
                    }
                    _ => {
                        let started_at = Instant::now();
                        let decision = provider_decision(
                            &prepared.provider,
                            &prepared.planning_request,
                            &prepared.tools,
                        )?;
                        (decision, Some(started_at.elapsed().as_millis() as u64))
                    }
                }
            } else {
                match preflight_decision {
                    Some(decision) => (decision, None),
                    None => {
                        let started_at = Instant::now();
                        let decision = provider_decision(
                            &prepared.provider,
                            &prepared.planning_request,
                            &prepared.tools,
                        )?;
                        (decision, Some(started_at.elapsed().as_millis() as u64))
                    }
                }
            };

        if let Some(tool_call) = first_decision.tool_call.take() {
            let normalized = normalize_tool_directive(
                tool_call,
                first_decision.assistant_message.take(),
                &first_decision.output_text,
                first_decision.reasoning_content.as_deref(),
                first_decision.reasoning_content_value.as_ref(),
            )?;
            first_decision.tool_call = Some(normalized.tool_call);
            first_decision.assistant_message = normalized.assistant_message;
        }

        if let Some(error) = provider_failure_message(
            &first_decision.provider_mode,
            first_decision.fallback_reason.as_deref(),
        ) {
            return Err(error);
        }

        let resolved_tool_call = self.resolve_tool_call(
            &prepared.user_message,
            prepared.retrieved.planner_history(),
            &prepared.planner_skills,
            first_decision.tool_call.clone(),
            !prepared.provider.requires_provider_native_tool_flow(),
        );
        let mut tool_selection_dispatch = self.dispatch_planner_hooks(
            PlannerHookPoint::ToolSelection,
            &self.build_planner_tool_selection_envelope(
                &prepared.user_message,
                prepared.retrieved.planner_history(),
                &prepared.planner_skills,
                first_decision.tool_call.as_ref(),
                resolved_tool_call.as_ref(),
            ),
            None,
            resolved_tool_call,
        );
        if let Some(error) = tool_selection_dispatch.fail_turn_error.take() {
            return Err(error);
        }
        if let Some(error) = tool_selection_dispatch.blocked_error.take() {
            return Err(error);
        }
        let resolved_tool_call = tool_selection_dispatch.selected_tool_call;
        planner_hook_trace_records.extend(tool_selection_dispatch.trace_records);
        planner_hook_trace_records.push(self.build_planner_tool_selection_trace_record(
            &prepared.user_message,
            first_decision.tool_call.as_ref(),
            resolved_tool_call.as_ref(),
        ));

        Ok(PlannedTurn {
            first_decision,
            resolved_tool_call,
            initial_decision_duration_ms,
            planner_hook_trace_records,
        })
    }

    fn build_planner_preflight_trace_record(
        &self,
        user_message: &str,
        planner_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        decision: Option<&ProviderDecision>,
    ) -> HookTraceRecord {
        let summary = match decision.and_then(|decision| decision.tool_call.as_ref()) {
            Some(tool_call) => format!(
                "planner preflight produced normalized provider decision with tool `{}`",
                tool_call.name
            ),
            None if decision.is_some() => {
                "planner preflight produced normalized provider decision without tool call"
                    .to_string()
            }
            None => "planner preflight deferred first decision to provider resolution".to_string(),
        };
        build_observe_hook_trace_record(
            "planner.preflight.observe",
            TurnHookPoint::PlannerTurnPreflight,
            1,
            summary,
            Some(format!(
                "message={} skills={}",
                preview_text(user_message, 64),
                planner_skills.len()
            )),
        )
    }

    fn build_planner_tool_selection_trace_record(
        &self,
        user_message: &str,
        provider_tool_call: Option<&ToolCall>,
        resolved_tool_call: Option<&ToolCall>,
    ) -> HookTraceRecord {
        let provider_tool = provider_tool_call
            .map(|tool_call| tool_call.name.as_str())
            .unwrap_or("none");
        let resolved_tool = resolved_tool_call
            .map(|tool_call| tool_call.name.as_str())
            .unwrap_or("none");
        let summary = if provider_tool == resolved_tool {
            format!(
                "planner tool selection kept normalized tool path `{}`",
                resolved_tool
            )
        } else {
            format!(
                "planner tool selection rewrote normalized tool path from `{}` to `{}`",
                provider_tool, resolved_tool
            )
        };
        build_observe_hook_trace_record(
            "planner.tool_selection.observe",
            TurnHookPoint::PlannerToolSelection,
            1,
            summary,
            Some(format!("message={}", preview_text(user_message, 64))),
        )
    }

    fn build_planner_preflight_envelope(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        decision: Option<&ProviderDecision>,
    ) -> PlannerFactsEnvelope {
        PlannerFactsEnvelope {
            session_id: None,
            run_id: None,
            hook_point: PlannerHookPoint::TurnPreflight,
            source_boundary: "runtime.plan_turn.preflight".to_string(),
            planner_source: "turn_planner.preflight_decision".to_string(),
            user_message_summary: Some(preview_text(user_message, 64)),
            history_turn_count: history.len(),
            available_skill_ids: available_skills
                .iter()
                .map(|skill| skill.skill_id.clone())
                .collect(),
            provider_decision_summary: decision.map(summarize_provider_decision),
            provider_tool_call_name: decision
                .and_then(|decision| decision.tool_call.as_ref())
                .map(|tool_call| tool_call.name.clone()),
            graph_goal_summary: None,
            graph_step_count: None,
            current_decision_summary: decision.map(summarize_provider_decision),
        }
    }

    fn build_planner_tool_selection_envelope(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        provider_tool_call: Option<&ToolCall>,
        selected_tool_call: Option<&ToolCall>,
    ) -> PlannerFactsEnvelope {
        PlannerFactsEnvelope {
            session_id: None,
            run_id: None,
            hook_point: PlannerHookPoint::ToolSelection,
            source_boundary: "runtime.plan_turn.tool_selection".to_string(),
            planner_source: "turn_planner.select_tool_call".to_string(),
            user_message_summary: Some(preview_text(user_message, 64)),
            history_turn_count: history.len(),
            available_skill_ids: available_skills
                .iter()
                .map(|skill| skill.skill_id.clone())
                .collect(),
            provider_decision_summary: provider_tool_call
                .map(|tool_call| format!("provider suggested `{}`", tool_call.name)),
            provider_tool_call_name: provider_tool_call.map(|tool_call| tool_call.name.clone()),
            graph_goal_summary: None,
            graph_step_count: None,
            current_decision_summary: selected_tool_call
                .map(|tool_call| format!("selected `{}`", tool_call.name)),
        }
    }

    fn build_planner_graph_decision_trace_record(
        &self,
        run: &GraphRun,
        decision: &GraphDecision,
    ) -> HookTraceRecord {
        build_observe_hook_trace_record(
            "planner.graph_decision.observe",
            TurnHookPoint::PlannerGraphDecision,
            1,
            format!(
                "planner graph decision produced `{}` for run `{}`",
                graph_decision_kind_label(&decision.kind),
                run.id
            ),
            Some(format!(
                "run_phase={:?} target_phase={:?} reason={:?}",
                run.phase, decision.target_phase, decision.reason
            )),
        )
    }

    fn build_planner_graph_decision_envelope(
        &self,
        run: &GraphRun,
        decision: &GraphDecision,
    ) -> PlannerFactsEnvelope {
        PlannerFactsEnvelope {
            session_id: run.session_id.clone(),
            run_id: Some(run.id.clone()),
            hook_point: PlannerHookPoint::GraphDecision,
            source_boundary: "runtime.decide_graph_after_turn_with_planner".to_string(),
            planner_source: "graph_planner.decide_after_turn".to_string(),
            user_message_summary: None,
            history_turn_count: run.steps.len(),
            available_skill_ids: Vec::new(),
            provider_decision_summary: None,
            provider_tool_call_name: None,
            graph_goal_summary: Some(preview_text(&run.goal, 96)),
            graph_step_count: Some(run.steps.len()),
            current_decision_summary: Some(decision.summary.clone()),
        }
    }

    pub fn record_planner_graph_decision_trace(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        run: &GraphRun,
        decision: &GraphDecision,
    ) -> Option<HookTraceRecord> {
        let record = self.build_planner_graph_decision_trace_record(run, decision);
        if self.append_turn_trace_hook_records(session_id, turn_id, vec![record.clone()]) {
            Some(record)
        } else {
            None
        }
    }

    fn build_capability_resolution_trace_record(
        &self,
        tool_call: &ToolCall,
        execution: &CapabilityToolExecutionResult,
    ) -> HookTraceRecord {
        let summary = match (
            execution.capability.as_ref(),
            execution.failure_kind.as_ref(),
        ) {
            (Some(capability), None) => format!(
                "capability mediation resolved `{}` to `{}`",
                tool_call.name, capability.capability_id
            ),
            (Some(capability), Some(failure)) => format!(
                "capability mediation resolved `{}` to `{}` but execution failed with `{}`",
                tool_call.name,
                capability.capability_id,
                failure.as_str()
            ),
            (None, Some(failure)) => format!(
                "capability mediation failed to resolve `{}`: {}",
                tool_call.name,
                failure.as_str()
            ),
            (None, None) => {
                format!(
                    "capability mediation observed `{}` without resolution details",
                    tool_call.name
                )
            }
        };
        build_observe_hook_trace_record(
            "capability.resolve.observe",
            TurnHookPoint::CapabilityResolve,
            1,
            summary,
            Some(format!("tool={}", tool_call.name)),
        )
    }

    fn build_skill_resolution_trace_record(
        &self,
        request: &SkillInvocationRequest,
        execution: &SkillToolExecutionResult,
    ) -> HookTraceRecord {
        let summary = match (execution.skill.as_ref(), execution.failure_layer.as_ref()) {
            (Some(skill), None) => format!(
                "skill mediation resolved `{}` with {} composed capability actions",
                skill.skill_id,
                execution.capability_executions.len()
            ),
            (Some(skill), Some(layer)) => format!(
                "skill mediation resolved `{}` but failed at `{}`",
                skill.skill_id,
                layer.as_str()
            ),
            (None, Some(layer)) => format!(
                "skill mediation failed to resolve `{}` at `{}`",
                request.skill_id,
                layer.as_str()
            ),
            (None, None) => {
                format!(
                    "skill mediation observed `{}` without resolution details",
                    request.skill_id
                )
            }
        };
        build_observe_hook_trace_record(
            "skill.tool_actions.observe",
            TurnHookPoint::SkillToolActionsResolve,
            1,
            summary,
            Some(format!("skill_id={}", request.skill_id)),
        )
    }

    fn build_capability_mediation_envelope(
        &self,
        tool_call: &ToolCall,
    ) -> CapabilityMediationEnvelope {
        let candidate_ids =
            candidate_capability_ids_for_tool_name(&self.capability_registry, &tool_call.name);
        let resolved_capability = candidate_ids
            .first()
            .and_then(|capability_id| self.capability_registry.inspect_capability(capability_id));
        CapabilityMediationEnvelope {
            session_id: None,
            run_id: None,
            hook_point: CapabilityMediationHookPoint::CapabilityResolve,
            source_boundary: "runtime.execute_registered_tool_call".to_string(),
            mediation_source: "capability_registry.resolve_tool_call".to_string(),
            requested_capability_id: resolved_capability
                .as_ref()
                .map(|capability| capability.capability_id.clone()),
            requested_skill_id: None,
            capability_kind: resolved_capability
                .as_ref()
                .map(|capability| capability.kind.as_str().to_string()),
            candidate_ids,
            argument_summary: tool_call.arguments.to_string(),
            source_id: resolved_capability
                .as_ref()
                .map(|capability| capability.source_id.clone()),
            source_kind: resolved_capability
                .as_ref()
                .map(|capability| capability.source_kind.as_str().to_string()),
        }
    }

    fn build_skill_mediation_envelope(
        &self,
        tool_call: &ToolCall,
        skill: &crate::agent::capability_bridge::SkillDescriptor,
    ) -> CapabilityMediationEnvelope {
        let source = self
            .capability_registry
            .inspect_skill_source(&skill.source_id);
        CapabilityMediationEnvelope {
            session_id: None,
            run_id: None,
            hook_point: CapabilityMediationHookPoint::SkillToolActionsResolve,
            source_boundary: "runtime.execute_registered_tool_call".to_string(),
            mediation_source: "capability_registry.resolve_skill_tool_actions".to_string(),
            requested_capability_id: None,
            requested_skill_id: Some(skill.skill_id.clone()),
            capability_kind: None,
            candidate_ids: skill.composed_capability_refs.clone(),
            argument_summary: tool_call.arguments.to_string(),
            source_id: Some(skill.source_id.clone()),
            source_kind: source.map(|source| source.source_kind.as_str().to_string()),
        }
    }

    fn build_mcp_source_ingress_envelope(
        &self,
        snapshot: &McpSourceSnapshot,
    ) -> CapabilityMediationEnvelope {
        CapabilityMediationEnvelope {
            session_id: None,
            run_id: None,
            hook_point: CapabilityMediationHookPoint::McpSourceIngress,
            source_boundary: "control_plane.apply_mcp_source_snapshot".to_string(),
            mediation_source: "capability_registry.replace_mcp_source_snapshot".to_string(),
            requested_capability_id: None,
            requested_skill_id: None,
            capability_kind: None,
            candidate_ids: snapshot
                .capabilities
                .iter()
                .map(|capability| capability.capability_id.clone())
                .collect(),
            argument_summary: "{}".to_string(),
            source_id: Some(snapshot.source.source_id.clone()),
            source_kind: Some(snapshot.source.source_kind.as_str().to_string()),
        }
    }

    fn build_skill_source_ingress_envelope(
        &self,
        snapshot: &SkillSourceSnapshot,
    ) -> CapabilityMediationEnvelope {
        CapabilityMediationEnvelope {
            session_id: None,
            run_id: None,
            hook_point: CapabilityMediationHookPoint::SkillSourceIngress,
            source_boundary: "control_plane.apply_skill_source_snapshot".to_string(),
            mediation_source: "capability_registry.replace_skill_source_snapshot".to_string(),
            requested_capability_id: None,
            requested_skill_id: None,
            capability_kind: None,
            candidate_ids: snapshot
                .skills
                .iter()
                .map(|skill| skill.skill_id.clone())
                .collect(),
            argument_summary: "{}".to_string(),
            source_id: Some(snapshot.source.source_id.clone()),
            source_kind: Some(snapshot.source.source_kind.as_str().to_string()),
        }
    }

    fn resolve_turn_images(
        &self,
        input: &TurnInput,
        retrieved: &RetrievedContextState,
        provider: &ProviderManager,
    ) -> Result<Vec<TurnInputImage>, String> {
        let mut images = input.images.clone();

        if images.is_empty()
            && provider.supports_image_input()
            && should_recall_recent_images(retrieved)
        {
            let recall_limit = recalled_image_limit(&retrieved.turn_context.user_message);
            images = self
                .sessions
                .load_recent_images(input.session_id.as_deref(), recall_limit);
        }

        validate_turn_images(&images)?;
        Ok(images)
    }

    fn persist_turn_outcome(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        provider_name: &str,
        provider_mode: &str,
        token_usage: Option<&TokenUsage>,
        provider_native_transcript: Option<Vec<Value>>,
        attachments: Vec<SessionAttachment>,
    ) -> PersistedTurnOutcome {
        let updated_session = self.sessions.append_turn(
            session_id,
            user_message,
            assistant_message,
            provider_native_transcript,
            attachments,
        );
        let retrieved = self.context_builder.retrieve_context_state(
            user_message,
            &[],
            &updated_session,
            None,
            None,
        );
        let session_summary = self.context_builder.build_session_summary(
            self.graph.name(),
            &retrieved,
            provider_name,
            Some(provider_mode),
        );
        let (input_tokens, cache_hit_input_tokens, reasoning_tokens, output_tokens, total_tokens) =
            token_usage_parts(token_usage);

        PersistedTurnOutcome {
            session_summary,
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_turn_trace(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        phase: &str,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        provider_meta: Option<&ProviderEventMeta>,
        provider_source: Option<String>,
        provider_mode: Option<String>,
        build_context_observation: Option<BuildContextObservation>,
        return_text: Option<String>,
        return_reasoning_content: Option<String>,
        fallback_reason: Option<String>,
        input_tokens: Option<u64>,
        cache_hit_input_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        session_summary: Option<String>,
        error: Option<String>,
    ) {
        self.persist_turn_trace_with_provider_calls(
            session_id,
            turn_id,
            user_message,
            phase,
            trace_steps,
            tool_activities,
            Vec::new(),
            provider_meta,
            provider_source,
            provider_mode,
            build_context_observation,
            return_text,
            return_reasoning_content,
            fallback_reason,
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            session_summary,
            error,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_turn_trace_with_provider_calls_and_hooks(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        phase: &str,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        provider_call_records: Vec<ProviderCallCacheRecord>,
        hook_trace_records: Vec<HookTraceRecord>,
        provider_meta: Option<&ProviderEventMeta>,
        provider_source: Option<String>,
        provider_mode: Option<String>,
        build_context_observation: Option<BuildContextObservation>,
        return_text: Option<String>,
        return_reasoning_content: Option<String>,
        fallback_reason: Option<String>,
        input_tokens: Option<u64>,
        cache_hit_input_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        session_summary: Option<String>,
        error: Option<String>,
    ) {
        let trace_timeline = build_persisted_trace_timeline(
            user_message,
            phase,
            provider_meta,
            provider_source.as_deref(),
            provider_mode.as_deref(),
            build_context_observation.as_ref(),
            &tool_activities,
            return_text.as_deref(),
            return_reasoning_content.as_deref(),
            fallback_reason.as_deref(),
            error.as_deref(),
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
        );
        self.sessions.record_turn_trace(
            session_id,
            TurnTraceRecord {
                turn_id: turn_id.to_string(),
                session_id: session_id.map(str::to_string),
                event_id: None,
                event_type: None,
                event_version: None,
                sequence: None,
                emitted_at_ms: None,
                title: build_turn_trace_title(user_message),
                phase: phase.to_string(),
                trace_steps,
                trace_timeline,
                tool_activities,
                provider_call_records,
                hook_trace_records,
                provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
                provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
                provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
                provider_model: provider_meta.map(|meta| meta.model.clone()),
                provider_source,
                provider_mode,
                build_context_observation,
                session_summary,
                fallback_reason,
                error,
                input_tokens,
                cache_hit_input_tokens,
                reasoning_tokens,
                output_tokens,
                total_tokens,
                first_token_latency_ms,
                turn_duration_ms,
                updated_at: 0,
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_turn_trace_with_provider_calls(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        phase: &str,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        provider_call_records: Vec<ProviderCallCacheRecord>,
        provider_meta: Option<&ProviderEventMeta>,
        provider_source: Option<String>,
        provider_mode: Option<String>,
        build_context_observation: Option<BuildContextObservation>,
        return_text: Option<String>,
        return_reasoning_content: Option<String>,
        fallback_reason: Option<String>,
        input_tokens: Option<u64>,
        cache_hit_input_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        output_tokens: Option<u64>,
        total_tokens: Option<u64>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        session_summary: Option<String>,
        error: Option<String>,
    ) {
        self.persist_turn_trace_with_provider_calls_and_hooks(
            session_id,
            turn_id,
            user_message,
            phase,
            trace_steps,
            tool_activities,
            provider_call_records,
            Vec::new(),
            provider_meta,
            provider_source,
            provider_mode,
            build_context_observation,
            return_text,
            return_reasoning_content,
            fallback_reason,
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            session_summary,
            error,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn fail_stream_turn_with_hook_dispatch(
        &mut self,
        sink: &impl TurnEventSink,
        control: &ExecutionControlRegistry,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        provider_meta: Option<&ProviderEventMeta>,
        build_context_observation: Option<BuildContextObservation>,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        provider_call_records: Vec<ProviderCallCacheRecord>,
        hook_trace_records: Vec<HookTraceRecord>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        completed_hops: usize,
        error: String,
    ) {
        self.update_execution_checkpoint(
            control,
            turn_id,
            "failed",
            provider_meta,
            completed_hops,
            None,
            &trace_steps,
            &tool_activities,
            None,
            None,
            None,
            Some("failed"),
            Some(&error),
        );
        self.persist_turn_trace_with_provider_calls_and_hooks(
            session_id,
            turn_id,
            user_message,
            "failed",
            trace_steps.clone(),
            tool_activities.clone(),
            provider_call_records.clone(),
            hook_trace_records.clone(),
            provider_meta,
            None,
            None,
            build_context_observation.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            turn_duration_ms,
            None,
            Some(error.clone()),
        );
        emit_stream_failed(
            sink,
            turn_id.to_string(),
            provider_meta,
            trace_steps,
            Some(tool_activities),
            first_token_latency_ms,
            turn_duration_ms,
            build_context_observation,
            None,
            Some(provider_call_records),
            Some(hook_trace_records),
            error,
            session_id.map(str::to_string),
        );
    }

    fn save_input_attachments(
        &mut self,
        input: &TurnInput,
    ) -> Result<Vec<SessionAttachment>, String> {
        self.save_input_attachments_for_session(input.session_id.as_deref(), &input.images)
    }

    fn save_input_attachments_for_session(
        &mut self,
        session_id: Option<&str>,
        images: &[TurnInputImage],
    ) -> Result<Vec<SessionAttachment>, String> {
        let Some(session_id) = session_id else {
            return Ok(Vec::new());
        };
        self.sessions.save_input_attachments(session_id, images)
    }

    fn persist_cancelled_turn_outcome(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        provider_meta: Option<&ProviderEventMeta>,
        attachments: Vec<SessionAttachment>,
    ) -> PersistedTurnOutcome {
        self.persist_turn_outcome(
            session_id,
            user_message,
            CANCELLED_TURN_MESSAGE,
            provider_meta
                .map(|meta| meta.provider_name.as_str())
                .unwrap_or("runtime"),
            "cancelled",
            None,
            None,
            attachments,
        )
    }

    fn fail_sync_turn_result(
        &self,
        provider_meta: Option<&ProviderEventMeta>,
        user_message: String,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        hook_trace_records: Vec<HookTraceRecord>,
        error: String,
    ) -> TurnResult {
        build_failed_turn_result_with_hooks(
            provider_meta,
            user_message,
            error,
            trace_steps,
            tool_activities,
            hook_trace_records,
        )
    }

    fn failed_trace_steps_for_tool_activities(
        &self,
        tool_activities: &[TurnToolActivity],
    ) -> Vec<TurnTraceStep> {
        if tool_activities.is_empty() {
            return self.telemetry_builder.failed_trace_before_tool();
        }
        let all_tools_ok = tool_activities
            .iter()
            .all(|activity| activity.status != "error");
        self.telemetry_builder.failed_trace_after_tool(all_tools_ok)
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_failed_sync_turn_trace_with_hooks(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        user_message: &str,
        provider_meta: Option<&ProviderEventMeta>,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        provider_call_records: Vec<ProviderCallCacheRecord>,
        hook_trace_records: Vec<HookTraceRecord>,
        provider_source: Option<String>,
        provider_mode: Option<String>,
        build_context_observation: Option<BuildContextObservation>,
        fallback_reason: Option<String>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        error: String,
    ) {
        self.persist_turn_trace_with_provider_calls_and_hooks(
            session_id,
            turn_id,
            user_message,
            "failed",
            trace_steps,
            tool_activities,
            provider_call_records,
            hook_trace_records,
            provider_meta,
            provider_source,
            provider_mode,
            build_context_observation,
            None,
            None,
            fallback_reason,
            None,
            None,
            None,
            None,
            None,
            first_token_latency_ms,
            turn_duration_ms,
            None,
            Some(error),
        );
    }

    fn annotate_sync_terminal_trace_with_envelope(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        envelope: &TurnEventEnvelope,
    ) {
        let _ = self.annotate_turn_trace_terminal_event(
            session_id,
            turn_id,
            Some(envelope.event_id.clone()),
            Some(envelope.event_type.clone()),
            Some(envelope.event_version.clone()),
            Some(envelope.sequence),
            Some(envelope.emitted_at_ms),
        );
    }

    fn apply_terminal_envelope_to_turn_result(
        &self,
        result: &mut TurnResult,
        envelope: &TurnEventEnvelope,
    ) {
        result.event_id = Some(envelope.event_id.clone());
        result.event_type = Some(envelope.event_type.clone());
        result.event_version = Some(envelope.event_version.clone());
        result.sequence = Some(envelope.sequence);
        result.emitted_at_ms = Some(envelope.emitted_at_ms);
    }

    fn update_execution_checkpoint(
        &self,
        control: &ExecutionControlRegistry,
        turn_id: &str,
        phase: &str,
        provider_meta: Option<&ProviderEventMeta>,
        completed_hops: usize,
        active_tool_name: Option<&str>,
        trace_steps: &[TurnTraceStep],
        tool_activities: &[TurnToolActivity],
        provider_source: Option<&str>,
        provider_mode: Option<&str>,
        fallback_reason: Option<&str>,
        status: Option<&str>,
        error: Option<&str>,
    ) {
        control.update(turn_id, |checkpoint| {
            checkpoint.phase = phase.to_string();
            checkpoint.completed_hops = completed_hops;
            checkpoint.max_hops = max_tool_hops_per_turn();
            checkpoint.active_tool_name = active_tool_name.map(str::to_string);
            checkpoint.trace_steps = trace_steps.to_vec();
            checkpoint.tool_activities = tool_activities.to_vec();
            checkpoint.provider_requested_name =
                provider_meta.map(|meta| meta.requested_name.clone());
            checkpoint.provider_name = provider_meta.map(|meta| meta.provider_name.clone());
            checkpoint.provider_protocol = provider_meta.map(|meta| meta.protocol.clone());
            checkpoint.provider_model = provider_meta.map(|meta| meta.model.clone());
            checkpoint.provider_source = provider_source.map(str::to_string);
            checkpoint.provider_mode = provider_mode.map(str::to_string);
            checkpoint.fallback_reason = fallback_reason.map(str::to_string);
            checkpoint.error = error.map(str::to_string);
            if let Some(status) = status {
                checkpoint.status = status.to_string();
            }
        });
    }

    fn should_cancel_turn(&self, control: &ExecutionControlRegistry, turn_id: &str) -> bool {
        control.is_stop_requested(turn_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn cancel_stream_turn<S: TurnEventSink>(
        &mut self,
        sink: &S,
        control: &ExecutionControlRegistry,
        turn_id: &str,
        session_id: Option<&str>,
        user_message: &str,
        input_images: &[TurnInputImage],
        provider_meta: Option<&ProviderEventMeta>,
        trace_steps: Vec<TurnTraceStep>,
        tool_activities: Vec<TurnToolActivity>,
        first_token_latency_ms: Option<u64>,
        turn_duration_ms: Option<u64>,
        build_context_observation: Option<BuildContextObservation>,
    ) {
        let error = "stopped_by_user".to_string();
        let attachments = self
            .save_input_attachments_for_session(session_id, input_images)
            .unwrap_or_default();
        let persisted = self.persist_cancelled_turn_outcome(
            session_id,
            user_message,
            provider_meta,
            attachments,
        );
        self.update_execution_checkpoint(
            control,
            turn_id,
            "cancelled",
            provider_meta,
            0,
            None,
            &trace_steps,
            &tool_activities,
            None,
            None,
            None,
            Some("cancelled"),
            Some(&error),
        );
        self.persist_turn_trace(
            session_id,
            turn_id,
            user_message,
            "cancelled",
            trace_steps.clone(),
            tool_activities.clone(),
            provider_meta,
            None,
            None,
            build_context_observation.clone(),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            first_token_latency_ms,
            turn_duration_ms,
            Some(persisted.session_summary),
            Some(error.clone()),
        );
        emit_stream_cancelled(
            sink,
            turn_id.to_string(),
            provider_meta,
            trace_steps,
            Some(tool_activities),
            first_token_latency_ms,
            turn_duration_ms,
            build_context_observation.clone(),
            Some(build_persisted_trace_timeline(
                user_message,
                "cancelled",
                provider_meta,
                None,
                None,
                build_context_observation.as_ref(),
                &[],
                None,
                None,
                None,
                Some(error.as_str()),
                None,
                None,
                None,
                None,
                None,
                first_token_latency_ms,
                turn_duration_ms,
            )),
            None,
            error,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_stream_tool_turn<S: TurnEventSink>(
        &mut self,
        sink: &S,
        control: &ExecutionControlRegistry,
        turn_id: &str,
        input: &TurnInput,
        user_message: &str,
        display_message: &str,
        provider: &ProviderManager,
        provider_meta: &ProviderEventMeta,
        tools: &[ToolDefinition],
        planning_request: &ProviderRequest,
        first_decision: &ProviderDecision,
        tool_call: ToolCall,
        initial_hook_trace_records: Vec<HookTraceRecord>,
        initial_turn_first_token_latency_ms: Option<u64>,
        turn_started_at: &Instant,
        provider_call_records: &mut Vec<ProviderCallCacheRecord>,
    ) {
        let context_observation = build_context_observation(planning_request, tools);
        let mut hop_records = Vec::new();
        let mut tool_activities = Vec::new();
        let mut current_tool_call = tool_call;
        let mut current_assistant_message = first_decision.assistant_message.clone();
        let mut current_assistant_output_text = first_decision.output_text.clone();
        let mut current_assistant_reasoning = first_decision.reasoning_content.clone();
        let mut current_assistant_reasoning_value = first_decision.reasoning_content_value.clone();
        let mut all_tools_ok = true;
        let mut completed_hops = 0usize;
        let mut accumulated_fallback_reason = first_decision.fallback_reason.clone();
        let mut accumulated_token_usage = first_decision.token_usage.clone();
        let first_token_latency = Rc::new(Cell::new(initial_turn_first_token_latency_ms));
        let mut hook_trace_records = initial_hook_trace_records;
        let mut seen_tool_signatures = BTreeSet::from([tool_call_signature(&current_tool_call)]);

        loop {
            completed_hops += 1;
            let trace_steps = self.telemetry_builder.trace_tool_active();
            self.update_execution_checkpoint(
                control,
                turn_id,
                "calling_tool",
                Some(provider_meta),
                completed_hops.saturating_sub(1),
                Some(current_tool_call.name.as_str()),
                &trace_steps,
                &tool_activities,
                None,
                None,
                accumulated_fallback_reason.as_deref(),
                Some("running"),
                None,
            );
            if self.should_cancel_turn(control, turn_id) {
                self.cancel_stream_turn(
                    sink,
                    control,
                    turn_id,
                    input.session_id.as_deref(),
                    display_message,
                    &input.images,
                    Some(provider_meta),
                    trace_steps,
                    tool_activities.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    Some(context_observation.clone()),
                );
                return;
            }
            emit_stream_event(
                sink,
                "turn:trace",
                turn_id.to_string(),
                "trace",
                Some("executing_tool"),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(trace_steps),
                Some(build_stream_progress_trace_timeline(
                    display_message,
                    provider_meta,
                    None,
                    None,
                    &context_observation,
                    &tool_activities,
                    Some(current_assistant_output_text.as_str()),
                    current_assistant_reasoning.as_deref(),
                    first_token_latency.get(),
                    "calling_tool",
                )),
                None,
                None,
                None,
                None,
                None,
            );

            let running_tool_activities = running_tool_activities_with_history(
                &tool_activities,
                self.telemetry_builder
                    .tool_activities_running(&current_tool_call),
            );
            let tool_start_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ToolCallStart);
            let tool_start_hook_trace_records = tool_start_hook_outcome.trace_records.clone();
            hook_trace_records.extend(tool_start_hook_trace_records.clone());
            emit_stream_event(
                sink,
                "turn:tool",
                turn_id.to_string(),
                "tool",
                Some("executing_tool"),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(build_stream_progress_trace_timeline(
                    display_message,
                    provider_meta,
                    None,
                    None,
                    &context_observation,
                    &running_tool_activities,
                    Some(current_assistant_output_text.as_str()),
                    current_assistant_reasoning.as_deref(),
                    first_token_latency.get(),
                    "calling_tool",
                )),
                Some(running_tool_activities),
                None,
                Some(tool_start_hook_trace_records),
                None,
                input.session_id.clone(),
            );
            if let Some(error) = tool_start_hook_outcome.fail_turn_error {
                let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    trace_steps,
                    running_tool_activities_with_history(
                        &tool_activities,
                        self.telemetry_builder
                            .tool_activities_running(&current_tool_call),
                    ),
                    provider_call_records.clone(),
                    tool_start_hook_outcome.trace_records,
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    completed_hops,
                    error,
                );
                return;
            }

            let (tool_result, invocation_record, capability_hook_trace_records) =
                self.execute_registered_tool_call(&current_tool_call);
            hook_trace_records.extend(capability_hook_trace_records);
            all_tools_ok &= tool_result.status == "ok";
            tool_activities.extend(annotate_capability_tool_activities(
                self.telemetry_builder
                    .tool_activities_after_result(&current_tool_call, &tool_result),
                invocation_record,
            ));
            let return_trace_steps = self.telemetry_builder.trace_return_active(all_tools_ok);
            self.update_execution_checkpoint(
                control,
                turn_id,
                "calling_model",
                Some(provider_meta),
                completed_hops,
                Some(current_tool_call.name.as_str()),
                &return_trace_steps,
                &tool_activities,
                None,
                None,
                accumulated_fallback_reason.as_deref(),
                Some("running"),
                None,
            );
            hop_records.push(ToolTurnHopRecord {
                assistant_output_text: current_assistant_output_text.clone(),
                assistant_reasoning_content: current_assistant_reasoning.clone(),
                assistant_reasoning_content_value: current_assistant_reasoning_value.clone(),
                tool_call: current_tool_call.clone(),
                tool_result: tool_result.clone(),
            });
            if self.should_cancel_turn(control, turn_id) {
                self.cancel_stream_turn(
                    sink,
                    control,
                    turn_id,
                    input.session_id.as_deref(),
                    display_message,
                    &input.images,
                    Some(provider_meta),
                    return_trace_steps.clone(),
                    tool_activities.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    Some(context_observation.clone()),
                );
                return;
            }

            let tool_end_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ToolCallEnd);
            let tool_end_hook_trace_records = tool_end_hook_outcome.trace_records.clone();
            hook_trace_records.extend(tool_end_hook_trace_records.clone());
            emit_stream_event(
                sink,
                "turn:tool",
                turn_id.to_string(),
                "tool",
                Some("tool_result_integrating"),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(build_stream_progress_trace_timeline(
                    display_message,
                    provider_meta,
                    None,
                    None,
                    &context_observation,
                    &tool_activities,
                    Some(current_assistant_output_text.as_str()),
                    current_assistant_reasoning.as_deref(),
                    first_token_latency.get(),
                    "calling_model",
                )),
                Some(tool_activities.clone()),
                None,
                Some(tool_end_hook_trace_records),
                None,
                input.session_id.clone(),
            );
            if let Some(error) = tool_end_hook_outcome.fail_turn_error {
                let trace_steps = self.telemetry_builder.failed_trace_after_tool(all_tools_ok);
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    trace_steps,
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    tool_end_hook_outcome.trace_records,
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    completed_hops,
                    error,
                );
                return;
            }

            emit_stream_event(
                sink,
                "turn:trace",
                turn_id.to_string(),
                "trace",
                Some("tool_result_integrating"),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(return_trace_steps),
                Some(build_stream_progress_trace_timeline(
                    display_message,
                    provider_meta,
                    None,
                    None,
                    &context_observation,
                    &tool_activities,
                    Some(current_assistant_output_text.as_str()),
                    current_assistant_reasoning.as_deref(),
                    first_token_latency.get(),
                    "calling_model",
                )),
                None,
                None,
                None,
                None,
                None,
            );

            if tool_result.status != "ok" {
                let error = build_tool_execution_error(
                    &current_tool_call.name,
                    tool_result.output.as_str(),
                );
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    self.telemetry_builder.failed_trace_after_tool(false),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    hook_trace_records.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    completed_hops,
                    error,
                );
                return;
            }

            let followup_model_hook_trace_records = self
                .dispatch_hook_trace_records(TurnHookPoint::ModelCallStart)
                .trace_records;
            emit_stream_event(
                sink,
                "turn:trace",
                turn_id.to_string(),
                "trace",
                Some("calling_model"),
                None,
                None,
                Some(provider_meta),
                None,
                None,
                None,
                Some(context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(self.telemetry_builder.trace_return_active(all_tools_ok)),
                Some(build_stream_progress_trace_timeline(
                    display_message,
                    provider_meta,
                    None,
                    None,
                    &context_observation,
                    &tool_activities,
                    Some(current_assistant_output_text.as_str()),
                    current_assistant_reasoning.as_deref(),
                    first_token_latency.get(),
                    "calling_model",
                )),
                Some(tool_activities.clone()),
                None,
                Some(followup_model_hook_trace_records),
                None,
                input.session_id.clone(),
            );

            let delta_turn_id = turn_id.to_string();
            let first_token_latency_for_emit = Rc::clone(&first_token_latency);
            let turn_started_at_for_latency = *turn_started_at;
            let supports_true_streaming_followup = provider.supports_true_streaming_followup();
            let provider_call_first_token_latency = Rc::new(Cell::new(None));
            let provider_call_first_token_latency_for_emit =
                Rc::clone(&provider_call_first_token_latency);
            let reasoning_batcher = Rc::new(RefCell::new(StreamReasoningBatcher::default()));
            let reasoning_batcher_for_emit = Rc::clone(&reasoning_batcher);
            let delta_turn_id_for_emit = delta_turn_id.clone();
            let provider_call_started_at = Instant::now();
            let response = match provider_followup_stream(
                provider,
                planning_request,
                tools,
                current_assistant_message.as_ref(),
                &current_tool_call,
                &tool_result,
                move |delta| {
                    let flush_delta = |text: Option<String>,
                                       reasoning_content: Option<String>,
                                       latency: Option<u64>| {
                        emit_stream_event(
                            sink,
                            "turn:delta",
                            delta_turn_id_for_emit.clone(),
                            "delta",
                            Some("calling_model"),
                            text,
                            reasoning_content,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            latency,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                        );
                    };
                    if supports_true_streaming_followup
                        && provider_call_first_token_latency_for_emit.get().is_none()
                    {
                        let value = provider_call_started_at.elapsed().as_millis() as u64;
                        provider_call_first_token_latency_for_emit.set(Some(value));
                    }
                    let latency = if supports_true_streaming_followup
                        && first_token_latency_for_emit.get().is_none()
                    {
                        let value = turn_started_at_for_latency.elapsed().as_millis() as u64;
                        first_token_latency_for_emit.set(Some(value));
                        Some(value)
                    } else {
                        None
                    };

                    match delta {
                        ProviderStreamChunk::Text(text) => {
                            if let Some(reasoning) = reasoning_batcher_for_emit.borrow_mut().flush()
                            {
                                flush_delta(None, Some(reasoning), latency);
                            }
                            flush_delta(Some(text), None, latency);
                        }
                        ProviderStreamChunk::Reasoning(reasoning) => {
                            if let Some(buffered_reasoning) =
                                reasoning_batcher_for_emit.borrow_mut().push(reasoning)
                            {
                                flush_delta(None, Some(buffered_reasoning), latency);
                            }
                        }
                    }
                },
            ) {
                Ok(response) => response,
                Err(error) => {
                    self.fail_stream_turn_with_hook_dispatch(
                        sink,
                        control,
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        Some(provider_meta),
                        Some(context_observation.clone()),
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities.clone(),
                        provider_call_records.clone(),
                        hook_trace_records.clone(),
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        completed_hops,
                        error,
                    );
                    return;
                }
            };
            if let Some(buffered_reasoning) = reasoning_batcher.borrow_mut().flush() {
                emit_stream_event(
                    sink,
                    "turn:delta",
                    delta_turn_id.clone(),
                    "delta",
                    Some("calling_model"),
                    None,
                    Some(buffered_reasoning.clone()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    input.session_id.clone(),
                );
            }
            let mut response = response;
            let provider_call_duration_ms = provider_call_started_at.elapsed().as_millis() as u64;
            let provider_call_used_true_stream =
                response.provider_source == "provider_followup_stream";
            provider_call_records.push(build_provider_call_cache_record(
                ProviderRequestKind::ToolFollowup,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                response.token_usage.as_ref(),
                if provider_call_used_true_stream {
                    provider_call_first_token_latency.get()
                } else {
                    None
                },
                Some(provider_call_duration_ms),
                if provider_call_used_true_stream {
                    ProviderLatencyKind::ProviderStream
                } else {
                    ProviderLatencyKind::BufferedResponse
                },
                Some(&context_observation),
            ));
            accumulated_token_usage =
                merge_token_usage(accumulated_token_usage, response.token_usage.as_ref());
            accumulated_fallback_reason = merge_fallback_reason(
                accumulated_fallback_reason,
                response.fallback_reason.clone(),
            );
            let return_trace_steps = self.telemetry_builder.trace_return_active(all_tools_ok);
            self.update_execution_checkpoint(
                control,
                turn_id,
                "calling_model",
                Some(provider_meta),
                completed_hops,
                None,
                &return_trace_steps,
                &tool_activities,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                accumulated_fallback_reason.as_deref(),
                Some("running"),
                None,
            );
            if self.should_cancel_turn(control, turn_id) {
                self.cancel_stream_turn(
                    sink,
                    control,
                    turn_id,
                    input.session_id.as_deref(),
                    display_message,
                    &input.images,
                    Some(provider_meta),
                    return_trace_steps.clone(),
                    tool_activities.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    Some(context_observation.clone()),
                );
                return;
            }

            if let Some(error) = provider_failure_message(
                &response.provider_mode,
                response.fallback_reason.as_deref(),
            ) {
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    hook_trace_records.clone(),
                    first_token_latency.get(),
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    completed_hops,
                    error,
                );
                return;
            }

            if let Some(next_tool_call) = response.tool_call.take() {
                let normalized = match normalize_tool_directive(
                    next_tool_call,
                    response.assistant_message.take(),
                    &response.output_text,
                    response.reasoning_content.as_deref(),
                    response.reasoning_content_value.as_ref(),
                ) {
                    Ok(normalized) => normalized,
                    Err(error) => {
                        self.fail_stream_turn_with_hook_dispatch(
                            sink,
                            control,
                            input.session_id.as_deref(),
                            turn_id,
                            display_message,
                            Some(provider_meta),
                            Some(context_observation.clone()),
                            self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                            tool_activities.clone(),
                            provider_call_records.clone(),
                            hook_trace_records.clone(),
                            first_token_latency.get(),
                            Some(turn_started_at.elapsed().as_millis() as u64),
                            completed_hops,
                            error,
                        );
                        return;
                    }
                };
                response.tool_call = Some(normalized.tool_call);
                response.assistant_message = normalized.assistant_message;
            }

            if let Some(next_tool_call) = response.tool_call.clone() {
                if completed_hops >= max_tool_followups_per_turn() {
                    let recovery = recover_tool_followup_completion_stream(
                        sink,
                        provider,
                        planning_request,
                        &user_message,
                        &hop_records,
                        &next_tool_call,
                        response.assistant_message.as_ref(),
                        &build_tool_followup_limit_error(max_tool_followups_per_turn()),
                        &context_observation,
                        turn_id,
                        input.session_id.clone(),
                        &first_token_latency,
                        turn_started_at,
                    );
                    provider_call_records.push(recovery.provider_call_record);
                    accumulated_token_usage = merge_token_usage(
                        accumulated_token_usage,
                        recovery.response.token_usage.as_ref(),
                    );
                    accumulated_fallback_reason = merge_fallback_reason(
                        accumulated_fallback_reason,
                        recovery.response.fallback_reason.clone(),
                    );
                    response = recovery.response;
                } else if completed_hops >= max_tool_hops_per_turn() {
                    self.fail_stream_turn_with_hook_dispatch(
                        sink,
                        control,
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        Some(provider_meta),
                        Some(context_observation.clone()),
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities.clone(),
                        provider_call_records.clone(),
                        hook_trace_records.clone(),
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        completed_hops,
                        build_tool_hop_limit_error(max_tool_hops_per_turn()),
                    );
                    return;
                } else {
                    let next_signature = tool_call_signature(&next_tool_call);
                    if !seen_tool_signatures.insert(next_signature) {
                        let recovery = recover_tool_followup_completion_stream(
                            sink,
                            provider,
                            planning_request,
                            &user_message,
                            &hop_records,
                            &next_tool_call,
                            response.assistant_message.as_ref(),
                            &build_duplicate_tool_call_error(&next_tool_call),
                            &context_observation,
                            turn_id,
                            input.session_id.clone(),
                            &first_token_latency,
                            turn_started_at,
                        );
                        provider_call_records.push(recovery.provider_call_record);
                        accumulated_token_usage = merge_token_usage(
                            accumulated_token_usage,
                            recovery.response.token_usage.as_ref(),
                        );
                        accumulated_fallback_reason = merge_fallback_reason(
                            accumulated_fallback_reason,
                            recovery.response.fallback_reason.clone(),
                        );
                        response = recovery.response;
                    } else {
                        current_assistant_message = response.assistant_message.clone();
                        current_assistant_output_text = response.output_text.clone();
                        current_assistant_reasoning = response.reasoning_content.clone();
                        current_assistant_reasoning_value =
                            response.reasoning_content_value.clone();
                        current_tool_call = next_tool_call;
                        continue;
                    }
                }
            }

            let completed_text = response.output_text.clone();
            let completed_mode = response.provider_mode.clone();
            let attachments = match self.save_input_attachments(input) {
                Ok(attachments) => attachments,
                Err(error) => {
                    self.fail_stream_turn_with_hook_dispatch(
                        sink,
                        control,
                        input.session_id.as_deref(),
                        turn_id,
                        display_message,
                        Some(provider_meta),
                        Some(context_observation.clone()),
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities.clone(),
                        provider_call_records.clone(),
                        hook_trace_records.clone(),
                        first_token_latency.get(),
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        completed_hops,
                        error,
                    );
                    return;
                }
            };
            let persisted = self.persist_turn_outcome(
                input.session_id.as_deref(),
                display_message,
                &completed_text,
                provider.name(),
                &completed_mode,
                accumulated_token_usage.as_ref(),
                native_transcript_for_tool_turn(user_message, &hop_records, &response),
                attachments,
            );
            let trace_steps = self
                .telemetry_builder
                .completed_trace_with_tool(all_tools_ok);
            let turn_duration_ms = Some(turn_started_at.elapsed().as_millis() as u64);
            self.update_execution_checkpoint(
                control,
                turn_id,
                "checkpointing",
                Some(provider_meta),
                completed_hops,
                None,
                &trace_steps,
                &tool_activities,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                accumulated_fallback_reason.as_deref(),
                Some("running"),
                None,
            );
            emit_stream_event(
                sink,
                "turn:phase_changed",
                turn_id.to_string(),
                "phase",
                Some("checkpointing"),
                None,
                None,
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                accumulated_fallback_reason.clone(),
                Some(context_observation.clone()),
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                turn_duration_ms,
                Some(trace_steps.clone()),
                None,
                Some(tool_activities.clone()),
                Some(provider_call_records.clone()),
                None,
                None,
                input.session_id.clone(),
            );
            let checkpoint_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::CheckpointPersistEnd);
            let checkpoint_hook_trace_records = checkpoint_hook_outcome.trace_records.clone();
            emit_stream_event(
                sink,
                "turn:checkpoint_persisted",
                turn_id.to_string(),
                "checkpoint",
                Some("checkpointing"),
                None,
                None,
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                accumulated_fallback_reason.clone(),
                Some(context_observation.clone()),
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                turn_duration_ms,
                Some(trace_steps.clone()),
                None,
                Some(tool_activities.clone()),
                Some(provider_call_records.clone()),
                Some(checkpoint_hook_trace_records.clone()),
                Some(persisted.session_summary.clone()),
                input.session_id.clone(),
            );
            if let Some(error) = checkpoint_hook_outcome.fail_turn_error {
                let mut checkpoint_terminal_hook_trace_records = hook_trace_records.clone();
                checkpoint_terminal_hook_trace_records
                    .extend(checkpoint_hook_trace_records.clone());
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    checkpoint_terminal_hook_trace_records,
                    first_token_latency.get(),
                    turn_duration_ms,
                    completed_hops,
                    error,
                );
                return;
            }
            let finalize_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::TurnFinalizeEnd);
            let finalize_hook_trace_records = finalize_hook_outcome.trace_records.clone();
            let mut terminal_hook_trace_records = hook_trace_records.clone();
            terminal_hook_trace_records.extend(checkpoint_hook_trace_records.clone());
            terminal_hook_trace_records.extend(finalize_hook_trace_records.clone());
            if let Some(error) = finalize_hook_outcome.fail_turn_error {
                self.fail_stream_turn_with_hook_dispatch(
                    sink,
                    control,
                    input.session_id.as_deref(),
                    turn_id,
                    display_message,
                    Some(provider_meta),
                    Some(context_observation.clone()),
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    terminal_hook_trace_records,
                    first_token_latency.get(),
                    turn_duration_ms,
                    completed_hops,
                    error,
                );
                return;
            }
            self.update_execution_checkpoint(
                control,
                turn_id,
                "ready",
                Some(provider_meta),
                completed_hops,
                None,
                &trace_steps,
                &tool_activities,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                accumulated_fallback_reason.as_deref(),
                Some("completed"),
                None,
            );
            self.persist_turn_trace_with_provider_calls_and_hooks(
                input.session_id.as_deref(),
                turn_id,
                display_message,
                "completed",
                trace_steps.clone(),
                tool_activities.clone(),
                provider_call_records.clone(),
                terminal_hook_trace_records.clone(),
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                Some(context_observation.clone()),
                Some(response.output_text.clone()),
                response.reasoning_content.clone(),
                accumulated_fallback_reason.clone(),
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                turn_duration_ms,
                Some(persisted.session_summary.clone()),
                None,
            );

            let completed_timeline = build_persisted_trace_timeline(
                display_message,
                "completed",
                Some(provider_meta),
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                Some(&context_observation),
                &tool_activities,
                Some(response.output_text.as_str()),
                response.reasoning_content.as_deref(),
                accumulated_fallback_reason.as_deref(),
                None,
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                turn_duration_ms,
            );
            emit_stream_event(
                sink,
                "turn:completed",
                turn_id.to_string(),
                "completed",
                Some("completed"),
                Some(response.output_text.clone()),
                response.reasoning_content.clone(),
                Some(provider_meta),
                Some(response.provider_source.clone()),
                Some(response.provider_mode.clone()),
                accumulated_fallback_reason.clone(),
                Some(context_observation.clone()),
                persisted.input_tokens,
                persisted.cache_hit_input_tokens,
                persisted.reasoning_tokens,
                persisted.output_tokens,
                persisted.total_tokens,
                first_token_latency.get(),
                turn_duration_ms,
                Some(trace_steps),
                Some(completed_timeline),
                Some(tool_activities),
                Some(provider_call_records.clone()),
                Some(terminal_hook_trace_records),
                Some(persisted.session_summary),
                input.session_id.clone(),
            );
            return;
        }
    }

    fn handle_sync_tool_turn(
        &mut self,
        user_message: String,
        display_message: String,
        provider: &ProviderManager,
        provider_meta: &ProviderEventMeta,
        tools: &[ToolDefinition],
        planning_request: &ProviderRequest,
        first_decision: &ProviderDecision,
        tool_call: ToolCall,
        initial_model_hook_trace_records: Vec<HookTraceRecord>,
        first_token_latency_ms: Option<u64>,
        _turn_started_at: &Instant,
        provider_call_records: &mut Vec<ProviderCallCacheRecord>,
    ) -> Result<SyncToolTurnOutcome, TurnResult> {
        let context_observation = build_context_observation(planning_request, tools);
        let mut hop_records = Vec::new();
        let mut tool_activities = Vec::new();
        let mut current_tool_call = tool_call;
        let mut current_assistant_message = first_decision.assistant_message.clone();
        let mut current_assistant_output_text = first_decision.output_text.clone();
        let mut current_assistant_reasoning = first_decision.reasoning_content.clone();
        let mut current_assistant_reasoning_value = first_decision.reasoning_content_value.clone();
        let mut all_tools_ok = true;
        let mut completed_hops = 0usize;
        let mut accumulated_fallback_reason = first_decision.fallback_reason.clone();
        let mut accumulated_token_usage = first_decision.token_usage.clone();
        let mut hook_trace_records = initial_model_hook_trace_records;
        let mut seen_tool_signatures = BTreeSet::from([tool_call_signature(&current_tool_call)]);

        loop {
            completed_hops += 1;
            runtime_log(format!(
                "turn:tool-execute hop={} name={} args={}",
                completed_hops, current_tool_call.name, current_tool_call.arguments
            ));
            let tool_start_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ToolCallStart);
            hook_trace_records.extend(tool_start_hook_outcome.trace_records.clone());
            if let Some(error) = tool_start_hook_outcome.fail_turn_error {
                return Err(self.fail_sync_turn_result(
                    Some(provider_meta),
                    display_message,
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    self.telemetry_builder
                        .tool_activities_running(&current_tool_call),
                    hook_trace_records,
                    error,
                ));
            }

            let (tool_result, invocation_record, capability_hook_trace_records) =
                self.execute_registered_tool_call(&current_tool_call);
            hook_trace_records.extend(capability_hook_trace_records);
            runtime_log(format!(
                "turn:tool-result hop={} name={} status={} output_preview={}",
                completed_hops,
                tool_result.tool_name,
                tool_result.status,
                preview_text(&tool_result.output, 160)
            ));
            all_tools_ok &= tool_result.status == "ok";
            tool_activities.extend(annotate_capability_tool_activities(
                self.telemetry_builder
                    .tool_activities_after_result(&current_tool_call, &tool_result),
                invocation_record,
            ));
            hop_records.push(ToolTurnHopRecord {
                assistant_output_text: current_assistant_output_text.clone(),
                assistant_reasoning_content: current_assistant_reasoning.clone(),
                assistant_reasoning_content_value: current_assistant_reasoning_value.clone(),
                tool_call: current_tool_call.clone(),
                tool_result: tool_result.clone(),
            });

            if tool_result.status != "ok" {
                return Err(self.fail_sync_turn_result(
                    Some(provider_meta),
                    display_message,
                    self.telemetry_builder.failed_trace_after_tool(false),
                    tool_activities,
                    hook_trace_records,
                    build_tool_execution_error(
                        &current_tool_call.name,
                        tool_result.output.as_str(),
                    ),
                ));
            }

            let tool_end_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ToolCallEnd);
            hook_trace_records.extend(tool_end_hook_outcome.trace_records.clone());
            if let Some(error) = tool_end_hook_outcome.fail_turn_error {
                return Err(self.fail_sync_turn_result(
                    Some(provider_meta),
                    display_message,
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities,
                    hook_trace_records,
                    error,
                ));
            }

            let provider_call_started_at = Instant::now();
            let response = match provider_followup(
                provider,
                planning_request,
                tools,
                current_assistant_message.as_ref(),
                &current_tool_call,
                &tool_result,
            ) {
                Ok(response) => response,
                Err(error) => {
                    return Err(build_failed_turn_result(
                        Some(provider_meta),
                        display_message,
                        error,
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities,
                    ));
                }
            };
            let mut response = response;
            let provider_call_duration_ms = provider_call_started_at.elapsed().as_millis() as u64;
            provider_call_records.push(build_provider_call_cache_record(
                ProviderRequestKind::ToolFollowup,
                Some(response.provider_source.as_str()),
                Some(response.provider_mode.as_str()),
                response.token_usage.as_ref(),
                None,
                Some(provider_call_duration_ms),
                ProviderLatencyKind::BufferedResponse,
                Some(&context_observation),
            ));
            accumulated_token_usage =
                merge_token_usage(accumulated_token_usage, response.token_usage.as_ref());
            accumulated_fallback_reason = merge_fallback_reason(
                accumulated_fallback_reason,
                response.fallback_reason.clone(),
            );

            if let Some(error) = provider_failure_message(
                &response.provider_mode,
                response.fallback_reason.as_deref(),
            ) {
                return Err(self.fail_sync_turn_result(
                    Some(provider_meta),
                    display_message,
                    self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                    tool_activities,
                    hook_trace_records,
                    error,
                ));
            }

            if let Some(next_tool_call) = response.tool_call.take() {
                let normalized = match normalize_tool_directive(
                    next_tool_call,
                    response.assistant_message.take(),
                    &response.output_text,
                    response.reasoning_content.as_deref(),
                    response.reasoning_content_value.as_ref(),
                ) {
                    Ok(normalized) => normalized,
                    Err(error) => {
                        return Err(self.fail_sync_turn_result(
                            Some(provider_meta),
                            display_message,
                            self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                            tool_activities,
                            hook_trace_records,
                            error,
                        ));
                    }
                };
                response.tool_call = Some(normalized.tool_call);
                response.assistant_message = normalized.assistant_message;
            }

            if let Some(next_tool_call) = response.tool_call.clone() {
                if completed_hops >= max_tool_followups_per_turn() {
                    let recovery = recover_tool_followup_completion(
                        provider,
                        planning_request,
                        &user_message,
                        &hop_records,
                        &next_tool_call,
                        response.assistant_message.as_ref(),
                        &build_tool_followup_limit_error(max_tool_followups_per_turn()),
                        &context_observation,
                    );
                    provider_call_records.push(recovery.provider_call_record);
                    accumulated_token_usage = merge_token_usage(
                        accumulated_token_usage,
                        recovery.response.token_usage.as_ref(),
                    );
                    accumulated_fallback_reason = merge_fallback_reason(
                        accumulated_fallback_reason,
                        recovery.response.fallback_reason.clone(),
                    );
                    response = recovery.response;
                } else if completed_hops >= max_tool_hops_per_turn() {
                    return Err(self.fail_sync_turn_result(
                        Some(provider_meta),
                        display_message,
                        self.telemetry_builder.failed_trace_after_tool(all_tools_ok),
                        tool_activities,
                        hook_trace_records,
                        build_tool_hop_limit_error(max_tool_hops_per_turn()),
                    ));
                } else {
                    let next_signature = tool_call_signature(&next_tool_call);
                    if !seen_tool_signatures.insert(next_signature) {
                        let recovery = recover_tool_followup_completion(
                            provider,
                            planning_request,
                            &user_message,
                            &hop_records,
                            &next_tool_call,
                            response.assistant_message.as_ref(),
                            &build_duplicate_tool_call_error(&next_tool_call),
                            &context_observation,
                        );
                        provider_call_records.push(recovery.provider_call_record);
                        accumulated_token_usage = merge_token_usage(
                            accumulated_token_usage,
                            recovery.response.token_usage.as_ref(),
                        );
                        accumulated_fallback_reason = merge_fallback_reason(
                            accumulated_fallback_reason,
                            recovery.response.fallback_reason.clone(),
                        );
                        response = recovery.response;
                    } else {
                        current_assistant_message = response.assistant_message.clone();
                        current_assistant_output_text = response.output_text.clone();
                        current_assistant_reasoning = response.reasoning_content.clone();
                        current_assistant_reasoning_value =
                            response.reasoning_content_value.clone();
                        current_tool_call = next_tool_call;
                        continue;
                    }
                }
            }

            return Ok(SyncToolTurnOutcome {
                assistant_message: response.output_text.clone(),
                provider_native_transcript: native_transcript_for_tool_turn(
                    &user_message,
                    &hop_records,
                    &response,
                ),
                provider_source: response.provider_source,
                provider_mode: response.provider_mode,
                fallback_reason: accumulated_fallback_reason,
                token_usage: accumulated_token_usage,
                trace_steps: self
                    .telemetry_builder
                    .completed_trace_with_tool(all_tools_ok),
                tool_activities,
                hook_trace_records,
                first_token_latency_ms,
            });
        }
    }

    pub fn run_turn(&mut self, input: TurnInput) -> TurnResult {
        let turn_started_at = Instant::now();
        let prepared = match self.prepare_turn(&input, false) {
            Ok(prepared) => prepared,
            Err(error) => {
                return build_failed_turn_result(
                    None,
                    String::new(),
                    error,
                    self.telemetry_builder.failed_trace_empty_input(),
                    Vec::new(),
                );
            }
        };

        runtime_log(format!(
            "turn:run requested={} provider={} protocol={} model={} message_preview={}",
            prepared.provider.requested_name(),
            prepared.provider.name(),
            prepared.provider.protocol_label(),
            prepared.provider.model(),
            preview_text(&prepared.user_message, 120)
        ));
        let provider_meta = provider_event_meta(&prepared.provider);
        let model_call_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::ModelCallStart);
        let initial_model_hook_trace_records = model_call_hook_outcome.trace_records.clone();
        if let Some(error) = model_call_hook_outcome.fail_turn_error {
            return self.fail_sync_turn_result(
                Some(&provider_meta),
                prepared.display_message,
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                model_call_hook_outcome.trace_records,
                error,
            );
        }

        let planned = match self.plan_turn(&prepared) {
            Ok(planned) => planned,
            Err(error) => {
                return self.fail_sync_turn_result(
                    Some(&provider_meta),
                    prepared.display_message,
                    self.telemetry_builder.failed_trace_before_tool(),
                    Vec::new(),
                    initial_model_hook_trace_records.clone(),
                    error,
                );
            }
        };

        let PlannedTurn {
            first_decision,
            resolved_tool_call,
            initial_decision_duration_ms,
            planner_hook_trace_records,
        } = planned;
        let PreparedTurn {
            user_message,
            display_message,
            provider,
            tools,
            planning_request,
            build_context_observation,
            ..
        } = prepared;
        let mut provider_call_records = vec![build_provider_call_cache_record(
            ProviderRequestKind::InitialRequest,
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            first_decision.token_usage.as_ref(),
            None,
            initial_decision_duration_ms,
            ProviderLatencyKind::BufferedResponse,
            Some(&build_context_observation),
        )];

        if let Some(error) = provider_failure_message(
            &first_decision.provider_mode,
            first_decision.fallback_reason.as_deref(),
        ) {
            let mut planning_hook_trace_records = initial_model_hook_trace_records.clone();
            planning_hook_trace_records.extend(planner_hook_trace_records.clone());
            return self.fail_sync_turn_result(
                Some(&provider_meta),
                display_message,
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                planning_hook_trace_records,
                error,
            );
        }
        let mut planning_hook_trace_records = initial_model_hook_trace_records.clone();
        planning_hook_trace_records.extend(planner_hook_trace_records.clone());
        let (
            assistant_message,
            provider_native_transcript,
            provider_source,
            provider_mode,
            fallback_reason,
            token_usage,
            trace_steps,
            tool_activities,
            mut hook_trace_records,
            first_token_latency_ms,
        ) = if let Some(tool_call) = resolved_tool_call {
            let initial_visible_first_token_latency_ms = None;
            match self.handle_sync_tool_turn(
                user_message.clone(),
                display_message.clone(),
                &provider,
                &provider_meta,
                &tools,
                &planning_request,
                &first_decision,
                tool_call,
                planning_hook_trace_records.clone(),
                initial_visible_first_token_latency_ms,
                &turn_started_at,
                &mut provider_call_records,
            ) {
                Ok(outcome) => (
                    outcome.assistant_message,
                    outcome.provider_native_transcript,
                    outcome.provider_source,
                    outcome.provider_mode,
                    outcome.fallback_reason,
                    outcome.token_usage,
                    outcome.trace_steps,
                    outcome.tool_activities,
                    outcome.hook_trace_records,
                    outcome.first_token_latency_ms,
                ),
                Err(failed_result) => return failed_result,
            }
        } else {
            (
                first_decision.output_text.clone(),
                native_transcript_for_completed_turn(
                    &user_message,
                    &first_decision,
                    provider.requires_provider_native_tool_flow(),
                ),
                first_decision.provider_source.clone(),
                first_decision.provider_mode.clone(),
                first_decision.fallback_reason.clone(),
                first_decision.token_usage.clone(),
                self.telemetry_builder.completed_trace_without_tool(),
                Vec::new(),
                planning_hook_trace_records.clone(),
                None,
            )
        };
        let attachments = match self.save_input_attachments(&input) {
            Ok(attachments) => attachments,
            Err(error) => {
                let failed_trace_steps =
                    self.failed_trace_steps_for_tool_activities(&tool_activities);
                let trace_turn_id = format!(
                    "sync:{}:{}",
                    input.session_id.as_deref().unwrap_or("local-dev-session"),
                    turn_started_at.elapsed().as_nanos()
                );
                self.persist_failed_sync_turn_trace_with_hooks(
                    input.session_id.as_deref(),
                    &trace_turn_id,
                    &display_message,
                    Some(&provider_meta),
                    failed_trace_steps.clone(),
                    tool_activities.clone(),
                    provider_call_records.clone(),
                    hook_trace_records.clone(),
                    Some(provider_source.clone()),
                    Some(provider_mode.clone()),
                    Some(build_context_observation.clone()),
                    fallback_reason.clone(),
                    first_token_latency_ms,
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    error.clone(),
                );
                let envelope = build_terminal_turn_event_envelope(
                    &trace_turn_id,
                    "turn:failed",
                    Some("failed"),
                    Some(&build_context_observation),
                    Some(&tool_activities),
                );
                self.annotate_sync_terminal_trace_with_envelope(
                    input.session_id.as_deref(),
                    &trace_turn_id,
                    &envelope,
                );
                let mut result = build_failed_turn_result_with_hooks(
                    Some(&provider_meta),
                    display_message,
                    error,
                    failed_trace_steps,
                    tool_activities,
                    hook_trace_records,
                );
                self.apply_terminal_envelope_to_turn_result(&mut result, &envelope);
                return result;
            }
        };
        let persisted = self.persist_turn_outcome(
            input.session_id.as_deref(),
            &display_message,
            &assistant_message,
            provider.name(),
            &provider_mode,
            token_usage.as_ref(),
            provider_native_transcript,
            attachments,
        );
        let turn_duration_ms = Some(turn_started_at.elapsed().as_millis() as u64);
        let checkpoint_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::CheckpointPersistEnd);
        hook_trace_records.extend(checkpoint_hook_outcome.trace_records.clone());
        if let Some(error) = checkpoint_hook_outcome.fail_turn_error {
            let failed_trace_steps = self.failed_trace_steps_for_tool_activities(&tool_activities);
            let trace_turn_id = format!(
                "sync:{}:{}",
                input.session_id.as_deref().unwrap_or("local-dev-session"),
                turn_started_at.elapsed().as_nanos()
            );
            self.persist_failed_sync_turn_trace_with_hooks(
                input.session_id.as_deref(),
                &trace_turn_id,
                &display_message,
                Some(&provider_meta),
                failed_trace_steps.clone(),
                tool_activities.clone(),
                provider_call_records.clone(),
                hook_trace_records.clone(),
                Some(provider_source.clone()),
                Some(provider_mode.clone()),
                Some(build_context_observation.clone()),
                fallback_reason.clone(),
                first_token_latency_ms,
                turn_duration_ms,
                error.clone(),
            );
            let envelope = build_terminal_turn_event_envelope(
                &trace_turn_id,
                "turn:failed",
                Some("failed"),
                Some(&build_context_observation),
                Some(&tool_activities),
            );
            self.annotate_sync_terminal_trace_with_envelope(
                input.session_id.as_deref(),
                &trace_turn_id,
                &envelope,
            );
            let mut result = self.fail_sync_turn_result(
                Some(&provider_meta),
                display_message,
                failed_trace_steps,
                tool_activities,
                hook_trace_records,
                error,
            );
            self.apply_terminal_envelope_to_turn_result(&mut result, &envelope);
            return result;
        }
        let finalize_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::TurnFinalizeEnd);
        hook_trace_records.extend(finalize_hook_outcome.trace_records.clone());
        if let Some(error) = finalize_hook_outcome.fail_turn_error {
            let failed_trace_steps = self.failed_trace_steps_for_tool_activities(&tool_activities);
            let trace_turn_id = format!(
                "sync:{}:{}",
                input.session_id.as_deref().unwrap_or("local-dev-session"),
                turn_started_at.elapsed().as_nanos()
            );
            self.persist_failed_sync_turn_trace_with_hooks(
                input.session_id.as_deref(),
                &trace_turn_id,
                &display_message,
                Some(&provider_meta),
                failed_trace_steps.clone(),
                tool_activities.clone(),
                provider_call_records.clone(),
                hook_trace_records.clone(),
                Some(provider_source.clone()),
                Some(provider_mode.clone()),
                Some(build_context_observation.clone()),
                fallback_reason.clone(),
                first_token_latency_ms,
                turn_duration_ms,
                error.clone(),
            );
            let envelope = build_terminal_turn_event_envelope(
                &trace_turn_id,
                "turn:failed",
                Some("failed"),
                Some(&build_context_observation),
                Some(&tool_activities),
            );
            self.annotate_sync_terminal_trace_with_envelope(
                input.session_id.as_deref(),
                &trace_turn_id,
                &envelope,
            );
            let mut result = self.fail_sync_turn_result(
                Some(&provider_meta),
                display_message,
                failed_trace_steps,
                tool_activities,
                hook_trace_records,
                error,
            );
            self.apply_terminal_envelope_to_turn_result(&mut result, &envelope);
            return result;
        }
        let trace_turn_id = format!(
            "sync:{}:{}",
            input.session_id.as_deref().unwrap_or("local-dev-session"),
            turn_started_at.elapsed().as_nanos()
        );
        self.persist_turn_trace_with_provider_calls_and_hooks(
            input.session_id.as_deref(),
            &trace_turn_id,
            &display_message,
            "completed",
            trace_steps.clone(),
            tool_activities.clone(),
            provider_call_records.clone(),
            hook_trace_records.clone(),
            Some(&provider_meta),
            Some(provider_source.clone()),
            Some(provider_mode.clone()),
            Some(build_context_observation.clone()),
            Some(assistant_message.clone()),
            None,
            fallback_reason.clone(),
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            Some(persisted.session_summary.clone()),
            None,
        );
        let terminal_envelope = build_terminal_turn_event_envelope(
            &trace_turn_id,
            "turn:completed",
            Some("completed"),
            Some(&build_context_observation),
            Some(&tool_activities),
        );
        self.annotate_sync_terminal_trace_with_envelope(
            input.session_id.as_deref(),
            &trace_turn_id,
            &terminal_envelope,
        );

        let trace_timeline = build_persisted_trace_timeline(
            display_message.as_str(),
            "completed",
            Some(&provider_meta),
            Some(provider_source.as_str()),
            Some(provider_mode.as_str()),
            Some(&build_context_observation),
            &tool_activities,
            Some(assistant_message.as_str()),
            None,
            fallback_reason.as_deref(),
            None,
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
        );

        let mut result = TurnResult {
            event_id: None,
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: "ready".to_string(),
            provider_requested_name: provider.requested_name().to_string(),
            provider_name: provider.name().to_string(),
            provider_protocol: provider.protocol_label().to_string(),
            provider_model: provider.model().to_string(),
            provider_source,
            provider_mode,
            fallback_reason,
            build_context_observation: Some(build_context_observation),
            input_tokens: persisted.input_tokens,
            cache_hit_input_tokens: persisted.cache_hit_input_tokens,
            reasoning_tokens: persisted.reasoning_tokens,
            output_tokens: persisted.output_tokens,
            total_tokens: persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms: Some(turn_started_at.elapsed().as_millis() as u64),
            user_message: display_message,
            assistant_message,
            trace_steps,
            trace_timeline,
            tool_activities,
            provider_call_records,
            hook_trace_records,
            session_summary: persisted.session_summary,
        };
        self.apply_terminal_envelope_to_turn_result(&mut result, &terminal_envelope);
        result
    }

    #[allow(dead_code)]
    pub fn start_turn_stream<S: TurnEventSink>(
        &mut self,
        sink: &S,
        turn_id: String,
        input: TurnInput,
    ) {
        let control = ExecutionControlRegistry::new();
        control.register_turn(&turn_id, input.session_id.as_deref(), None);
        self.start_turn_stream_with_control(sink, &control, turn_id, input);
    }

    pub fn start_turn_stream_with_control<S: TurnEventSink>(
        &mut self,
        sink: &S,
        control: &ExecutionControlRegistry,
        turn_id: String,
        input: TurnInput,
    ) {
        let turn_started_at = Instant::now();
        let prepared = match self.prepare_turn(&input, true) {
            Ok(prepared) => prepared,
            Err(error) => {
                emit_turn_failed(
                    sink,
                    turn_id,
                    None,
                    None,
                    None,
                    None,
                    self.telemetry_builder.failed_trace_empty_input(),
                    error,
                    input.session_id.clone(),
                );
                return;
            }
        };
        runtime_log(format!(
            "turn:start id={} requested={} provider={} protocol={} model={} message_preview={}",
            turn_id,
            prepared.provider.requested_name(),
            prepared.provider.name(),
            prepared.provider.protocol_label(),
            prepared.provider.model(),
            preview_text(&prepared.user_message, 120)
        ));
        let prepared_provider_meta = provider_event_meta(&prepared.provider);
        let start_trace_steps = self.telemetry_builder.start_trace_steps();
        self.update_execution_checkpoint(
            control,
            &turn_id,
            "calling_model",
            Some(&prepared_provider_meta),
            0,
            None,
            &start_trace_steps,
            &[],
            None,
            None,
            None,
            Some("running"),
            None,
        );

        emit_stream_event(
            sink,
            "turn:started",
            turn_id.clone(),
            "started",
            Some("calling_model"),
            Some(prepared.user_message.clone()),
            None,
            Some(&prepared_provider_meta),
            None,
            None,
            None,
            Some(prepared.build_context_observation.clone()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(start_trace_steps.clone()),
            Some(build_stream_started_trace_timeline(
                prepared.user_message.as_str(),
                &prepared_provider_meta,
                &prepared.build_context_observation,
            )),
            None,
            None,
            None,
            None,
            input.session_id.clone(),
        );
        if self.should_cancel_turn(control, &turn_id) {
            self.cancel_stream_turn(
                sink,
                control,
                &turn_id,
                input.session_id.as_deref(),
                prepared.display_message.as_str(),
                &input.images,
                Some(&prepared_provider_meta),
                start_trace_steps,
                Vec::new(),
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                Some(prepared.build_context_observation.clone()),
            );
            return;
        }

        let preflight_decision = self.planner.preflight_decision(
            &prepared.user_message,
            prepared.retrieved.planner_history(),
            &prepared.planner_skills,
        );
        let mut preflight_dispatch = self.dispatch_planner_hooks(
            PlannerHookPoint::TurnPreflight,
            &self.build_planner_preflight_envelope(
                &prepared.user_message,
                prepared.retrieved.planner_history(),
                &prepared.planner_skills,
                preflight_decision.as_ref(),
            ),
            preflight_decision,
            None,
        );
        if let Some(error) = preflight_dispatch.fail_turn_error.take() {
            let trace_steps = self.telemetry_builder.failed_trace_before_tool();
            self.update_execution_checkpoint(
                control,
                &turn_id,
                "failed",
                Some(&prepared_provider_meta),
                0,
                None,
                &trace_steps,
                &[],
                None,
                None,
                None,
                Some("failed"),
                Some(&error),
            );
            self.persist_turn_trace_with_provider_calls_and_hooks(
                input.session_id.as_deref(),
                &turn_id,
                &prepared.display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                Vec::new(),
                preflight_dispatch.trace_records,
                Some(&prepared_provider_meta),
                None,
                None,
                Some(prepared.build_context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                None,
                Some(error.clone()),
            );
            emit_turn_failed(
                sink,
                turn_id,
                Some(prepared.provider.requested_name().to_string()),
                Some(prepared.provider.name().to_string()),
                Some(prepared.provider.protocol_label().to_string()),
                Some(prepared.provider.model().to_string()),
                trace_steps,
                error,
                input.session_id.clone(),
            );
            return;
        }
        if let Some(error) = preflight_dispatch.blocked_error.take() {
            let trace_steps = self.telemetry_builder.failed_trace_before_tool();
            self.update_execution_checkpoint(
                control,
                &turn_id,
                "failed",
                Some(&prepared_provider_meta),
                0,
                None,
                &trace_steps,
                &[],
                None,
                None,
                None,
                Some("failed"),
                Some(&error),
            );
            self.persist_turn_trace_with_provider_calls_and_hooks(
                input.session_id.as_deref(),
                &turn_id,
                &prepared.display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                Vec::new(),
                preflight_dispatch.trace_records,
                Some(&prepared_provider_meta),
                None,
                None,
                Some(prepared.build_context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                None,
                Some(error.clone()),
            );
            emit_turn_failed(
                sink,
                turn_id,
                Some(prepared.provider.requested_name().to_string()),
                Some(prepared.provider.name().to_string()),
                Some(prepared.provider.protocol_label().to_string()),
                Some(prepared.provider.model().to_string()),
                trace_steps,
                error,
                input.session_id.clone(),
            );
            return;
        }
        let preflight_decision = preflight_dispatch.decision;
        let mut planner_hook_trace_records = preflight_dispatch.trace_records;
        planner_hook_trace_records.push(self.build_planner_preflight_trace_record(
            &prepared.user_message,
            &prepared.planner_skills,
            preflight_decision.as_ref(),
        ));
        let supports_true_streaming_decision = prepared.provider.supports_true_streaming_decision();
        let turn_id_for_stream = turn_id.clone();
        let stream_session_id = input.session_id.clone();
        let pending_model_call_fail_turn =
            Rc::new(RefCell::new(None::<(Vec<HookTraceRecord>, String)>));
        let pending_model_call_fail_turn_for_stream = Rc::clone(&pending_model_call_fail_turn);
        let stream_initial_decision = || -> Result<
            (
                ProviderDecision,
                Option<u64>,
                Option<u64>,
                Option<u64>,
                ProviderLatencyKind,
            ),
            String,
        > {
            let model_call_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ModelCallStart);
            let model_call_hook_trace_records = model_call_hook_outcome.trace_records.clone();
            emit_stream_event(
                sink,
                "turn:trace",
                turn_id.clone(),
                "trace",
                Some("calling_model"),
                None,
                None,
                Some(&prepared_provider_meta),
                None,
                None,
                None,
                Some(prepared.build_context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(start_trace_steps.clone()),
                Some(build_stream_progress_trace_timeline(
                    prepared.display_message.as_str(),
                    &prepared_provider_meta,
                    None,
                    None,
                    &prepared.build_context_observation,
                    &[],
                    None,
                    None,
                    None,
                    "calling_model",
                )),
                None,
                None,
                Some(model_call_hook_trace_records),
                None,
                input.session_id.clone(),
            );
            if let Some(error) = model_call_hook_outcome.fail_turn_error {
                *pending_model_call_fail_turn_for_stream.borrow_mut() =
                    Some((model_call_hook_outcome.trace_records, error));
                return Err(HOOK_FAILTURN_HANDLED_SENTINEL.to_string());
            }
            let initial_decision_started_at = Instant::now();
            let initial_turn_first_token_latency = Rc::new(Cell::new(None));
            let initial_call_first_token_latency = Rc::new(Cell::new(None));
            let initial_turn_first_token_latency_for_emit =
                Rc::clone(&initial_turn_first_token_latency);
            let initial_call_first_token_latency_for_emit =
                Rc::clone(&initial_call_first_token_latency);
            let reasoning_batcher = Rc::new(RefCell::new(StreamReasoningBatcher::default()));
            let reasoning_batcher_for_emit = Rc::clone(&reasoning_batcher);
            let turn_id_for_delta_emit = turn_id_for_stream.clone();
            let stream_session_id_for_delta_emit = stream_session_id.clone();

            let decision = provider_decision_stream(
                &prepared.provider,
                &prepared.planning_request,
                &prepared.tools,
                move |delta| {
                    let flush_delta = |text: Option<String>,
                                       reasoning_content: Option<String>,
                                       turn_latency: Option<u64>| {
                        emit_stream_event(
                            sink,
                            "turn:delta",
                            turn_id_for_delta_emit.clone(),
                            "delta",
                            Some("calling_model"),
                            text,
                            reasoning_content,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            turn_latency,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            None,
                            stream_session_id_for_delta_emit.clone(),
                        );
                    };
                    if initial_call_first_token_latency_for_emit.get().is_none() {
                        let value = initial_decision_started_at.elapsed().as_millis() as u64;
                        initial_call_first_token_latency_for_emit.set(Some(value));
                    }
                    let turn_latency =
                        if initial_turn_first_token_latency_for_emit.get().is_none() {
                            let value = turn_started_at.elapsed().as_millis() as u64;
                            initial_turn_first_token_latency_for_emit.set(Some(value));
                            Some(value)
                    } else {
                        None
                    };
                    match delta {
                        ProviderStreamChunk::Text(text) => {
                            if let Some(reasoning) = reasoning_batcher_for_emit.borrow_mut().flush() {
                                flush_delta(None, Some(reasoning), turn_latency);
                            }
                            flush_delta(Some(text), None, turn_latency);
                        }
                        ProviderStreamChunk::Reasoning(reasoning) => {
                            if let Some(buffered_reasoning) =
                                reasoning_batcher_for_emit.borrow_mut().push(reasoning)
                            {
                                flush_delta(None, Some(buffered_reasoning), turn_latency);
                            }
                        }
                    }
                },
            )?;
            if let Some(buffered_reasoning) = reasoning_batcher.borrow_mut().flush() {
                emit_stream_event(
                    sink,
                    "turn:delta",
                    turn_id_for_stream.clone(),
                    "delta",
                    Some("calling_model"),
                    None,
                    Some(buffered_reasoning.clone()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    input.session_id.clone(),
                );
            }
            Ok((
                decision,
                Some(initial_decision_started_at.elapsed().as_millis() as u64),
                initial_turn_first_token_latency.get(),
                initial_call_first_token_latency.get(),
                ProviderLatencyKind::ProviderStream,
            ))
        };
        let pending_model_call_fail_turn_for_sync = Rc::clone(&pending_model_call_fail_turn);
        let decide_sync = || -> Result<
            (
                ProviderDecision,
                Option<u64>,
                Option<u64>,
                Option<u64>,
                ProviderLatencyKind,
            ),
            String,
        > {
            let model_call_hook_outcome =
                self.dispatch_hook_trace_records(TurnHookPoint::ModelCallStart);
            let model_call_hook_trace_records = model_call_hook_outcome.trace_records.clone();
            emit_stream_event(
                sink,
                "turn:trace",
                turn_id.clone(),
                "trace",
                Some("calling_model"),
                None,
                None,
                Some(&prepared_provider_meta),
                None,
                None,
                None,
                Some(prepared.build_context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(start_trace_steps.clone()),
                Some(build_stream_progress_trace_timeline(
                    prepared.display_message.as_str(),
                    &prepared_provider_meta,
                    None,
                    None,
                    &prepared.build_context_observation,
                    &[],
                    None,
                    None,
                    None,
                    "calling_model",
                )),
                None,
                None,
                Some(model_call_hook_trace_records),
                None,
                input.session_id.clone(),
            );
            if let Some(error) = model_call_hook_outcome.fail_turn_error {
                *pending_model_call_fail_turn_for_sync.borrow_mut() =
                    Some((model_call_hook_outcome.trace_records, error));
                return Err(HOOK_FAILTURN_HANDLED_SENTINEL.to_string());
            }
            let started_at = Instant::now();
            let decision = provider_decision(
                &prepared.provider,
                &prepared.planning_request,
                &prepared.tools,
            )?;
            Ok((
                decision,
                Some(started_at.elapsed().as_millis() as u64),
                None,
                None,
                ProviderLatencyKind::BufferedResponse,
            ))
        };
        let planned = (|| -> Result<
            (
                ProviderDecision,
                Option<u64>,
                Option<u64>,
                Option<u64>,
                ProviderLatencyKind,
            ),
            String,
        > {
            if prepared.provider.requires_provider_native_tool_flow() {
                return match preflight_decision {
                    Some(decision) if planner_decision_can_override_native_tool_flow(&decision) => {
                        Ok((decision, None, None, None, ProviderLatencyKind::Unknown))
                    }
                    _ => {
                        if supports_true_streaming_decision {
                            stream_initial_decision().or_else(|_| decide_sync())
                        } else {
                            decide_sync()
                        }
                    }
                };
            }

            match preflight_decision {
                Some(decision) => Ok((decision, None, None, None, ProviderLatencyKind::Unknown)),
                None => {
                    if supports_true_streaming_decision {
                        stream_initial_decision().or_else(|_| decide_sync())
                    } else {
                        decide_sync()
                    }
                }
            }
        })();
        let (
            mut first_decision,
            initial_decision_duration_ms,
            initial_turn_first_token_latency_ms,
            initial_call_first_token_latency_ms,
            initial_latency_kind,
        ) = match planned {
            Ok(result) => result,
            Err(error) => {
                if error == HOOK_FAILTURN_HANDLED_SENTINEL {
                    if let Some((hook_trace_records, hook_error)) =
                        pending_model_call_fail_turn.borrow_mut().take()
                    {
                        let trace_steps = self.telemetry_builder.failed_trace_before_tool();
                        self.fail_stream_turn_with_hook_dispatch(
                            sink,
                            control,
                            input.session_id.as_deref(),
                            &turn_id,
                            &prepared.display_message,
                            Some(&prepared_provider_meta),
                            Some(prepared.build_context_observation.clone()),
                            trace_steps,
                            Vec::new(),
                            Vec::new(),
                            hook_trace_records,
                            None,
                            Some(turn_started_at.elapsed().as_millis() as u64),
                            0,
                            hook_error,
                        );
                    }
                    return;
                }
                let trace_steps = self.telemetry_builder.failed_trace_before_tool();
                self.update_execution_checkpoint(
                    control,
                    &turn_id,
                    "failed",
                    Some(&prepared_provider_meta),
                    0,
                    None,
                    &trace_steps,
                    &[],
                    None,
                    None,
                    None,
                    Some("failed"),
                    Some(&error),
                );
                self.persist_turn_trace(
                    input.session_id.as_deref(),
                    &turn_id,
                    &prepared.display_message,
                    "failed",
                    trace_steps.clone(),
                    Vec::new(),
                    Some(&prepared_provider_meta),
                    None,
                    None,
                    Some(prepared.build_context_observation.clone()),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    None,
                    Some(error.clone()),
                );
                emit_turn_failed(
                    sink,
                    turn_id,
                    Some(prepared.provider.requested_name().to_string()),
                    Some(prepared.provider.name().to_string()),
                    Some(prepared.provider.protocol_label().to_string()),
                    Some(prepared.provider.model().to_string()),
                    trace_steps,
                    error,
                    input.session_id.clone(),
                );
                return;
            }
        };
        if let Some(tool_call) = first_decision.tool_call.take() {
            let normalized = match normalize_tool_directive(
                tool_call,
                first_decision.assistant_message.take(),
                &first_decision.output_text,
                first_decision.reasoning_content.as_deref(),
                first_decision.reasoning_content_value.as_ref(),
            ) {
                Ok(normalized) => normalized,
                Err(error) => {
                    let trace_steps = self.telemetry_builder.failed_trace_before_tool();
                    self.update_execution_checkpoint(
                        control,
                        &turn_id,
                        "failed",
                        Some(&prepared_provider_meta),
                        0,
                        None,
                        &trace_steps,
                        &[],
                        None,
                        None,
                        None,
                        Some("failed"),
                        Some(&error),
                    );
                    self.persist_turn_trace(
                        input.session_id.as_deref(),
                        &turn_id,
                        &prepared.display_message,
                        "failed",
                        trace_steps.clone(),
                        Vec::new(),
                        Some(&prepared_provider_meta),
                        None,
                        None,
                        Some(prepared.build_context_observation.clone()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        initial_turn_first_token_latency_ms,
                        Some(turn_started_at.elapsed().as_millis() as u64),
                        None,
                        Some(error.clone()),
                    );
                    emit_turn_failed(
                        sink,
                        turn_id,
                        Some(prepared.provider.requested_name().to_string()),
                        Some(prepared.provider.name().to_string()),
                        Some(prepared.provider.protocol_label().to_string()),
                        Some(prepared.provider.model().to_string()),
                        trace_steps,
                        error,
                        input.session_id.clone(),
                    );
                    return;
                }
            };
            first_decision.tool_call = Some(normalized.tool_call);
            first_decision.assistant_message = normalized.assistant_message;
        }
        let resolved_tool_call = self.resolve_tool_call(
            &prepared.user_message,
            prepared.retrieved.planner_history(),
            &prepared.planner_skills,
            first_decision.tool_call.clone(),
            !prepared.provider.requires_provider_native_tool_flow(),
        );
        let mut tool_selection_dispatch = self.dispatch_planner_hooks(
            PlannerHookPoint::ToolSelection,
            &self.build_planner_tool_selection_envelope(
                &prepared.user_message,
                prepared.retrieved.planner_history(),
                &prepared.planner_skills,
                first_decision.tool_call.as_ref(),
                resolved_tool_call.as_ref(),
            ),
            None,
            resolved_tool_call,
        );
        if let Some(error) = tool_selection_dispatch.fail_turn_error.take() {
            let trace_steps = self.telemetry_builder.failed_trace_before_tool();
            self.update_execution_checkpoint(
                control,
                &turn_id,
                "failed",
                Some(&prepared_provider_meta),
                0,
                None,
                &trace_steps,
                &[],
                None,
                None,
                None,
                Some("failed"),
                Some(&error),
            );
            self.persist_turn_trace_with_provider_calls_and_hooks(
                input.session_id.as_deref(),
                &turn_id,
                &prepared.display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                Vec::new(),
                {
                    let mut records = planner_hook_trace_records.clone();
                    records.extend(tool_selection_dispatch.trace_records);
                    records
                },
                Some(&prepared_provider_meta),
                None,
                None,
                Some(prepared.build_context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                initial_turn_first_token_latency_ms,
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                None,
                Some(error.clone()),
            );
            emit_turn_failed(
                sink,
                turn_id,
                Some(prepared.provider.requested_name().to_string()),
                Some(prepared.provider.name().to_string()),
                Some(prepared.provider.protocol_label().to_string()),
                Some(prepared.provider.model().to_string()),
                trace_steps,
                error,
                input.session_id.clone(),
            );
            return;
        }
        if let Some(error) = tool_selection_dispatch.blocked_error.take() {
            let trace_steps = self.telemetry_builder.failed_trace_before_tool();
            self.update_execution_checkpoint(
                control,
                &turn_id,
                "failed",
                Some(&prepared_provider_meta),
                0,
                None,
                &trace_steps,
                &[],
                None,
                None,
                None,
                Some("failed"),
                Some(&error),
            );
            self.persist_turn_trace_with_provider_calls_and_hooks(
                input.session_id.as_deref(),
                &turn_id,
                &prepared.display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                Vec::new(),
                {
                    let mut records = planner_hook_trace_records.clone();
                    records.extend(tool_selection_dispatch.trace_records);
                    records
                },
                Some(&prepared_provider_meta),
                None,
                None,
                Some(prepared.build_context_observation.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                initial_turn_first_token_latency_ms,
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                None,
                Some(error.clone()),
            );
            emit_turn_failed(
                sink,
                turn_id,
                Some(prepared.provider.requested_name().to_string()),
                Some(prepared.provider.name().to_string()),
                Some(prepared.provider.protocol_label().to_string()),
                Some(prepared.provider.model().to_string()),
                trace_steps,
                error,
                input.session_id.clone(),
            );
            return;
        }
        let resolved_tool_call = tool_selection_dispatch.selected_tool_call;
        planner_hook_trace_records.extend(tool_selection_dispatch.trace_records);
        planner_hook_trace_records.push(self.build_planner_tool_selection_trace_record(
            &prepared.user_message,
            first_decision.tool_call.as_ref(),
            resolved_tool_call.as_ref(),
        ));
        let PreparedTurn {
            user_message,
            display_message,
            provider,
            tools,
            planning_request,
            build_context_observation,
            ..
        } = prepared;
        let provider_meta = provider_event_meta(&provider);
        let mut provider_call_records = vec![build_provider_call_cache_record(
            ProviderRequestKind::InitialRequest,
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            first_decision.token_usage.as_ref(),
            if initial_latency_kind == ProviderLatencyKind::ProviderStream {
                initial_call_first_token_latency_ms
            } else {
                None
            },
            initial_decision_duration_ms,
            initial_latency_kind.clone(),
            Some(&build_context_observation),
        )];

        if let Some(error) = provider_failure_message(
            &first_decision.provider_mode,
            first_decision.fallback_reason.as_deref(),
        ) {
            let trace_steps = self.telemetry_builder.failed_trace_before_tool();
            self.update_execution_checkpoint(
                control,
                &turn_id,
                "failed",
                Some(&provider_meta),
                0,
                None,
                &trace_steps,
                &[],
                Some(first_decision.provider_source.as_str()),
                Some(first_decision.provider_mode.as_str()),
                first_decision.fallback_reason.as_deref(),
                Some("failed"),
                Some(&error),
            );
            self.persist_turn_trace_with_provider_calls(
                input.session_id.as_deref(),
                &turn_id,
                &display_message,
                "failed",
                trace_steps.clone(),
                Vec::new(),
                provider_call_records.clone(),
                Some(&provider_meta),
                Some(first_decision.provider_source.clone()),
                Some(first_decision.provider_mode.clone()),
                Some(build_context_observation.clone()),
                None,
                None,
                first_decision.fallback_reason.clone(),
                None,
                None,
                None,
                None,
                None,
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                None,
                Some(error.clone()),
            );
            emit_stream_failed(
                sink,
                turn_id,
                Some(&provider_meta),
                trace_steps,
                None,
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                Some(build_context_observation.clone()),
                None,
                Some(provider_call_records.clone()),
                None,
                error,
                input.session_id.clone(),
            );
            return;
        }

        if self.should_cancel_turn(control, &turn_id) {
            self.cancel_stream_turn(
                sink,
                control,
                &turn_id,
                input.session_id.as_deref(),
                &display_message,
                &input.images,
                Some(&provider_meta),
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                Some(build_context_observation.clone()),
            );
            return;
        }

        if let Some(tool_call) = resolved_tool_call {
            self.handle_stream_tool_turn(
                sink,
                control,
                &turn_id,
                &input,
                &user_message,
                &display_message,
                &provider,
                &provider_meta,
                &tools,
                &planning_request,
                &first_decision,
                tool_call,
                planner_hook_trace_records.clone(),
                initial_turn_first_token_latency_ms,
                &turn_started_at,
                &mut provider_call_records,
            );
            return;
        }

        emit_stream_event(
            sink,
            "turn:trace",
            turn_id.clone(),
            "trace",
            Some("response_ready"),
            None,
            None,
            None,
            Some(first_decision.provider_source.clone()),
            None,
            None,
            Some(build_context_observation.clone()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(self.telemetry_builder.trace_return_active_without_tool()),
            Some(build_stream_progress_trace_timeline(
                display_message.as_str(),
                &provider_meta,
                Some(first_decision.provider_source.as_str()),
                Some(first_decision.provider_mode.as_str()),
                &build_context_observation,
                &[],
                None,
                None,
                if initial_latency_kind == ProviderLatencyKind::ProviderStream {
                    initial_call_first_token_latency_ms
                } else {
                    None
                },
                "response_ready",
            )),
            None,
            None,
            None,
            None,
            input.session_id.clone(),
        );
        self.update_execution_checkpoint(
            control,
            &turn_id,
            "calling_model",
            Some(&provider_meta),
            0,
            None,
            &self.telemetry_builder.trace_return_active_without_tool(),
            &[],
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            first_decision.fallback_reason.as_deref(),
            Some("running"),
            None,
        );
        if self.should_cancel_turn(control, &turn_id) {
            self.cancel_stream_turn(
                sink,
                control,
                &turn_id,
                input.session_id.as_deref(),
                &display_message,
                &input.images,
                Some(&provider_meta),
                self.telemetry_builder.trace_return_active_without_tool(),
                Vec::new(),
                None,
                Some(turn_started_at.elapsed().as_millis() as u64),
                Some(build_context_observation.clone()),
            );
            return;
        }

        let first_token_latency_ms = if initial_latency_kind == ProviderLatencyKind::ProviderStream
        {
            initial_turn_first_token_latency_ms
        } else {
            let first_visible_first_token_latency_ms =
                if let Some(reasoning_content) = first_decision.reasoning_content.as_deref() {
                    stream_reasoning_chunks(
                        sink,
                        &turn_id,
                        "calling_model",
                        reasoning_content,
                        &turn_started_at,
                        None,
                        false,
                    )
                } else {
                    None
                };
            stream_text_chunks(
                sink,
                &turn_id,
                "calling_model",
                &first_decision.output_text,
                &turn_started_at,
                first_visible_first_token_latency_ms,
                false,
            )
        };
        let completed_text = first_decision.output_text.clone();
        let completed_mode = first_decision.provider_mode.clone();
        let attachments = match self.save_input_attachments(&input) {
            Ok(attachments) => attachments,
            Err(error) => {
                let trace_steps = self.telemetry_builder.failed_trace_before_tool();
                self.update_execution_checkpoint(
                    control,
                    &turn_id,
                    "failed",
                    Some(&provider_meta),
                    0,
                    None,
                    &trace_steps,
                    &[],
                    Some(first_decision.provider_source.as_str()),
                    Some(first_decision.provider_mode.as_str()),
                    first_decision.fallback_reason.as_deref(),
                    Some("failed"),
                    Some(&error),
                );
                self.persist_turn_trace_with_provider_calls(
                    input.session_id.as_deref(),
                    &turn_id,
                    &display_message,
                    "failed",
                    trace_steps.clone(),
                    Vec::new(),
                    provider_call_records.clone(),
                    Some(&provider_meta),
                    Some(first_decision.provider_source.clone()),
                    Some(first_decision.provider_mode.clone()),
                    Some(build_context_observation.clone()),
                    None,
                    None,
                    first_decision.fallback_reason.clone(),
                    None,
                    None,
                    None,
                    None,
                    None,
                    first_token_latency_ms,
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    None,
                    Some(error.clone()),
                );
                emit_stream_failed(
                    sink,
                    turn_id,
                    Some(&provider_meta),
                    trace_steps,
                    None,
                    first_token_latency_ms,
                    Some(turn_started_at.elapsed().as_millis() as u64),
                    Some(build_context_observation.clone()),
                    None,
                    Some(provider_call_records.clone()),
                    None,
                    error,
                    input.session_id.clone(),
                );
                return;
            }
        };
        let persisted = self.persist_turn_outcome(
            input.session_id.as_deref(),
            &display_message,
            &completed_text,
            provider.name(),
            &completed_mode,
            first_decision.token_usage.as_ref(),
            native_transcript_for_completed_turn(
                &user_message,
                &first_decision,
                provider.requires_provider_native_tool_flow(),
            ),
            attachments,
        );
        let trace_steps = self.telemetry_builder.completed_trace_without_tool();
        let turn_duration_ms = Some(turn_started_at.elapsed().as_millis() as u64);
        self.update_execution_checkpoint(
            control,
            &turn_id,
            "checkpointing",
            Some(&provider_meta),
            0,
            None,
            &trace_steps,
            &[],
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            first_decision.fallback_reason.as_deref(),
            Some("running"),
            None,
        );
        emit_stream_event(
            sink,
            "turn:phase_changed",
            turn_id.clone(),
            "phase",
            Some("checkpointing"),
            None,
            None,
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            first_decision.fallback_reason.clone(),
            Some(build_context_observation.clone()),
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            Some(trace_steps.clone()),
            None,
            Some(Vec::new()),
            Some(provider_call_records.clone()),
            None,
            None,
            input.session_id.clone(),
        );
        let checkpoint_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::CheckpointPersistEnd);
        let checkpoint_hook_trace_records = checkpoint_hook_outcome.trace_records.clone();
        emit_stream_event(
            sink,
            "turn:checkpoint_persisted",
            turn_id.clone(),
            "checkpoint",
            Some("checkpointing"),
            None,
            None,
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            first_decision.fallback_reason.clone(),
            Some(build_context_observation.clone()),
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            Some(trace_steps.clone()),
            None,
            Some(Vec::new()),
            Some(provider_call_records.clone()),
            Some(checkpoint_hook_trace_records.clone()),
            Some(persisted.session_summary.clone()),
            input.session_id.clone(),
        );
        if let Some(error) = checkpoint_hook_outcome.fail_turn_error {
            let mut checkpoint_terminal_hook_trace_records = planner_hook_trace_records.clone();
            checkpoint_terminal_hook_trace_records.extend(checkpoint_hook_trace_records.clone());
            self.fail_stream_turn_with_hook_dispatch(
                sink,
                control,
                input.session_id.as_deref(),
                &turn_id,
                &display_message,
                Some(&provider_meta),
                Some(build_context_observation.clone()),
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                provider_call_records.clone(),
                checkpoint_terminal_hook_trace_records,
                first_token_latency_ms,
                turn_duration_ms,
                0,
                error,
            );
            return;
        }
        let finalize_hook_outcome =
            self.dispatch_hook_trace_records(TurnHookPoint::TurnFinalizeEnd);
        let finalize_hook_trace_records = finalize_hook_outcome.trace_records.clone();
        let mut terminal_hook_trace_records = planner_hook_trace_records.clone();
        terminal_hook_trace_records.extend(checkpoint_hook_trace_records.clone());
        terminal_hook_trace_records.extend(finalize_hook_trace_records.clone());
        if let Some(error) = finalize_hook_outcome.fail_turn_error {
            self.fail_stream_turn_with_hook_dispatch(
                sink,
                control,
                input.session_id.as_deref(),
                &turn_id,
                &display_message,
                Some(&provider_meta),
                Some(build_context_observation.clone()),
                self.telemetry_builder.failed_trace_before_tool(),
                Vec::new(),
                provider_call_records.clone(),
                terminal_hook_trace_records,
                first_token_latency_ms,
                turn_duration_ms,
                0,
                error,
            );
            return;
        }
        self.update_execution_checkpoint(
            control,
            &turn_id,
            "ready",
            Some(&provider_meta),
            0,
            None,
            &trace_steps,
            &[],
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            first_decision.fallback_reason.as_deref(),
            Some("completed"),
            None,
        );
        self.persist_turn_trace_with_provider_calls_and_hooks(
            input.session_id.as_deref(),
            &turn_id,
            &display_message,
            "completed",
            trace_steps.clone(),
            Vec::new(),
            provider_call_records.clone(),
            terminal_hook_trace_records.clone(),
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            Some(build_context_observation.clone()),
            Some(first_decision.output_text.clone()),
            first_decision.reasoning_content.clone(),
            first_decision.fallback_reason.clone(),
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
            Some(persisted.session_summary.clone()),
            None,
        );

        let completed_timeline = build_persisted_trace_timeline(
            display_message.as_str(),
            "completed",
            Some(&provider_meta),
            Some(first_decision.provider_source.as_str()),
            Some(first_decision.provider_mode.as_str()),
            Some(&build_context_observation),
            &[],
            Some(first_decision.output_text.as_str()),
            first_decision.reasoning_content.as_deref(),
            first_decision.fallback_reason.as_deref(),
            None,
            persisted.input_tokens,
            persisted.cache_hit_input_tokens,
            persisted.reasoning_tokens,
            persisted.output_tokens,
            persisted.total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
        );
        emit_stream_event(
            sink,
            "turn:completed",
            turn_id,
            "completed",
            Some("completed"),
            Some(first_decision.output_text.clone()),
            first_decision.reasoning_content.clone(),
            Some(&provider_meta),
            Some(first_decision.provider_source.clone()),
            Some(first_decision.provider_mode.clone()),
            first_decision.fallback_reason.clone(),
            Some(build_context_observation.clone()),
            Some(persisted.input_tokens).flatten(),
            Some(persisted.cache_hit_input_tokens).flatten(),
            Some(persisted.reasoning_tokens).flatten(),
            Some(persisted.output_tokens).flatten(),
            Some(persisted.total_tokens).flatten(),
            first_token_latency_ms,
            turn_duration_ms,
            Some(trace_steps),
            Some(completed_timeline),
            Some(Vec::new()),
            Some(provider_call_records),
            Some(terminal_hook_trace_records),
            Some(persisted.session_summary),
            input.session_id.clone(),
        );
    }

    fn resolve_provider(&self, input: &TurnInput) -> ProviderManager {
        let mut selection = self
            .provider_resolver
            .resolve_provider_selection(input.provider_id.as_deref(), input.model_id.as_deref());

        if selection.capabilities.supports_reasoning {
            selection.reasoning_effort = input.reasoning_effort.clone();
        } else {
            selection.reasoning_effort = None;
        }

        ProviderManager::new(selection)
    }

    fn resolve_tool_call(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        provider_tool_call: Option<ToolCall>,
        allow_local_fallback: bool,
    ) -> Option<ToolCall> {
        if allow_local_fallback {
            self.planner.select_tool_call(
                user_message,
                history,
                available_skills,
                provider_tool_call,
            )
        } else {
            provider_tool_call
        }
    }

    fn execute_capability_tool_call(&self, tool_call: &ToolCall) -> CapabilityToolExecutionResult {
        let action = match self.capability_registry.resolve_tool_call(tool_call) {
            Ok(action) => action,
            Err(failure_kind) => {
                runtime_log(format!(
                    "turn:capability-resolve-failure tool={} class={}",
                    tool_call.name,
                    failure_kind.as_str()
                ));
                return self
                    .capability_registry
                    .capability_failure_result(tool_call, failure_kind);
            }
        };

        runtime_log(format!(
            "turn:capability-resolved capability_id={} kind={} mode={}",
            action.capability.capability_id,
            action.capability.kind.as_str(),
            action.capability.invocation_mode.as_str()
        ));

        let tool_result = self.tool_executor.execute(&action.tool_call);
        let failure_kind = if tool_result.status == "ok" {
            None
        } else {
            Some(CapabilityFailureKind::InvocationFailed)
        };

        CapabilityToolExecutionResult {
            capability: Some(action.capability),
            tool_call: action.tool_call,
            tool_result,
            failure_kind,
        }
    }

    fn execute_registered_tool_call(
        &self,
        tool_call: &ToolCall,
    ) -> (
        crate::agent::tools::ToolResult,
        crate::agent::telemetry::CapabilityInvocationRecord,
        Vec<HookTraceRecord>,
    ) {
        if let Some(skill) = self
            .capability_registry
            .match_executable_skill_tool_name(&tool_call.name)
        {
            let mediation_envelope = self.build_skill_mediation_envelope(tool_call, &skill);
            let mediation = self.dispatch_capability_mediation_hooks(
                CapabilityMediationHookPoint::SkillToolActionsResolve,
                &mediation_envelope,
            );
            if let Some(error) = mediation.fail_turn_error {
                return (
                    blocked_tool_result(tool_call, &error),
                    build_blocked_skill_invocation_record(tool_call, Some(&skill), &error),
                    mediation.trace_records,
                );
            }
            if let Some(error) = mediation.blocked_error {
                return (
                    blocked_tool_result(tool_call, &error),
                    build_blocked_skill_invocation_record(tool_call, Some(&skill), &error),
                    mediation.trace_records,
                );
            }

            let execution = self.execute_skill_tool_call(&SkillInvocationRequest {
                skill_id: skill.skill_id.clone(),
                arguments: mediation.arguments.clone(),
            });
            let mut hook_trace_records = mediation.trace_records;
            hook_trace_records.push(self.build_skill_resolution_trace_record(
                &SkillInvocationRequest {
                    skill_id: skill.skill_id.clone(),
                    arguments: mediation.arguments,
                },
                &execution,
            ));
            let invocation_record = execution
                .capability_executions
                .first()
                .map(|result| {
                    result.invocation_record_with_skill_context(
                        execution.skill.as_ref(),
                        execution.failure_layer.as_ref(),
                    )
                })
                .unwrap_or(crate::agent::telemetry::CapabilityInvocationRecord {
                    tool_name: tool_call.name.clone(),
                    capability_id: None,
                    source_id: None,
                    source_kind: None,
                    capability_kind: None,
                    invocation_mode: None,
                    failure_kind: None,
                    requires_approval: None,
                    host_mediated: None,
                    permission_scope: None,
                    skill_id: execution
                        .skill
                        .as_ref()
                        .map(|descriptor| descriptor.skill_id.clone()),
                    skill_source_id: execution
                        .skill
                        .as_ref()
                        .map(|descriptor| descriptor.source_id.clone()),
                    composed_capability_refs: execution
                        .skill
                        .as_ref()
                        .map(|descriptor| descriptor.composed_capability_refs.clone()),
                    composed_capability_kinds: execution.skill.as_ref().map(|descriptor| {
                        descriptor
                            .composed_capability_kinds
                            .iter()
                            .map(|kind| kind.as_str().to_string())
                            .collect()
                    }),
                    failure_layer: execution
                        .failure_layer
                        .as_ref()
                        .map(|layer| layer.as_str().to_string()),
                });

            let tool_result = build_skill_tool_result(tool_call, &execution);
            return (tool_result, invocation_record, hook_trace_records);
        }

        let mediation_envelope = self.build_capability_mediation_envelope(tool_call);
        let mediation = self.dispatch_capability_mediation_hooks(
            CapabilityMediationHookPoint::CapabilityResolve,
            &mediation_envelope,
        );
        if let Some(error) = mediation.fail_turn_error {
            return (
                blocked_tool_result(tool_call, &error),
                build_blocked_capability_invocation_record(tool_call, &error),
                mediation.trace_records,
            );
        }
        if let Some(error) = mediation.blocked_error {
            return (
                blocked_tool_result(tool_call, &error),
                build_blocked_capability_invocation_record(tool_call, &error),
                mediation.trace_records,
            );
        }
        let execution = self.execute_capability_tool_call(&ToolCall {
            arguments: mediation.arguments,
            ..tool_call.clone()
        });
        let invocation_record = execution.invocation_record();
        let mut hook_trace_records = mediation.trace_records;
        hook_trace_records
            .push(self.build_capability_resolution_trace_record(tool_call, &execution));
        (execution.tool_result, invocation_record, hook_trace_records)
    }

    fn execute_skill_tool_call(
        &self,
        request: &SkillInvocationRequest,
    ) -> SkillToolExecutionResult {
        let (skill, actions) = match self.capability_registry.resolve_skill_tool_actions(request) {
            Ok(resolved) => resolved,
            Err(failure_layer) => {
                runtime_log(format!(
                    "turn:skill-resolve-failure skill_id={} layer={}",
                    request.skill_id,
                    failure_layer.as_str()
                ));
                return self
                    .capability_registry
                    .skill_failure_result(request, failure_layer);
            }
        };

        runtime_log(format!(
            "turn:skill-resolved skill_id={} source_id={} composed_refs={} kinds={}",
            skill.skill_id,
            skill.source_id,
            skill.composed_capability_refs.join(","),
            skill
                .composed_capability_kinds
                .iter()
                .map(|kind| kind.as_str())
                .collect::<Vec<_>>()
                .join(",")
        ));

        let mut capability_executions = Vec::with_capacity(actions.len());
        let mut failure_layer = None;

        for action in actions {
            let tool_result = self.tool_executor.execute(&action.tool_call);
            let capability_failure = if tool_result.status == "ok" {
                None
            } else {
                failure_layer = Some(SkillFailureLayer::UnderlyingCapabilityExecution);
                Some(CapabilityFailureKind::InvocationFailed)
            };
            capability_executions.push(CapabilityToolExecutionResult {
                capability: Some(action.capability),
                tool_call: action.tool_call,
                tool_result,
                failure_kind: capability_failure,
            });
            if failure_layer.is_some() {
                break;
            }
        }

        SkillToolExecutionResult {
            skill: Some(skill),
            capability_executions,
            failure_layer,
        }
    }
}

fn candidate_capability_ids_for_tool_name(
    registry: &CapabilityRegistry,
    tool_name: &str,
) -> Vec<String> {
    let mut candidate_ids = Vec::new();
    let raw = tool_name.trim();
    if raw.is_empty() {
        return candidate_ids;
    }

    candidate_ids.push(format!("builtin:{raw}"));
    let canonical = raw.replace('.', "_");
    if canonical != raw {
        candidate_ids.push(format!("builtin:{canonical}"));
    }

    for capability in registry.list_capabilities(None, Some("tool")) {
        if capability.label == raw && !candidate_ids.contains(&capability.capability_id) {
            candidate_ids.push(capability.capability_id);
        }
    }

    candidate_ids
}

fn normalized_arguments_from_summary(summary: &str) -> Value {
    serde_json::from_str(summary).unwrap_or_else(|_| Value::Object(Map::new()))
}

fn apply_capability_argument_patches(
    hook_point: &CapabilityMediationHookPoint,
    original_argument_summary: &str,
    execution_results: &[crate::agent::hooks::HookExecutionResult],
) -> Result<Value, String> {
    let merge_outcome = crate::agent::hooks::merge_patch_results(
        execution_results,
        HookPatchConflictPolicy::LastWriteWins,
    )?;
    let mut arguments = normalized_arguments_from_summary(original_argument_summary);

    for operation in merge_outcome.operations {
        if !crate::agent::hooks::capability_mediation_transform_operation_allowed(
            hook_point,
            &operation.operation,
        ) {
            return Err(format!(
                "hook `{}` attempted non-whitelisted capability mediation patch `{}`",
                operation.hook_name, operation.operation.path
            ));
        }
        arguments = apply_arguments_patch(arguments, &operation.operation)?;
    }

    Ok(arguments)
}

fn apply_arguments_patch(
    arguments: Value,
    operation: &HookPatchOperation,
) -> Result<Value, String> {
    if operation.path != "request.arguments" {
        return Err(format!(
            "unsupported capability mediation patch path `{}`",
            operation.path
        ));
    }

    match operation.operation {
        HookPatchOperationKind::Set => {
            let value = parse_hook_patch_value(operation)?;
            Ok(value)
        }
        HookPatchOperationKind::Merge => {
            let value = parse_hook_patch_value(operation)?;
            match (arguments, value) {
                (Value::Object(mut existing), Value::Object(incoming)) => {
                    for (key, value) in incoming {
                        existing.insert(key, value);
                    }
                    Ok(Value::Object(existing))
                }
                (_, other) => Ok(other),
            }
        }
        HookPatchOperationKind::Remove => Ok(Value::Object(Map::new())),
    }
}

fn parse_hook_patch_value(operation: &HookPatchOperation) -> Result<Value, String> {
    let Some(value_text) = operation.value_text.as_deref() else {
        return Err(format!(
            "hook patch on `{}` requires value_text for capability mediation",
            operation.path
        ));
    };
    serde_json::from_str(value_text).map_err(|error| {
        format!(
            "invalid capability mediation patch payload for `{}`: {error}",
            operation.path
        )
    })
}

fn summarize_provider_decision(decision: &ProviderDecision) -> String {
    match decision.tool_call.as_ref() {
        Some(tool_call) => format!("decision tool `{}`", tool_call.name),
        None => "decision without tool call".to_string(),
    }
}

fn apply_planner_patches(
    hook_point: &PlannerHookPoint,
    mut decision: Option<ProviderDecision>,
    mut selected_tool_call: Option<ToolCall>,
    execution_results: &[crate::agent::hooks::HookExecutionResult],
) -> Result<(Option<ProviderDecision>, Option<ToolCall>), String> {
    let merge_outcome = crate::agent::hooks::merge_patch_results(
        execution_results,
        HookPatchConflictPolicy::LastWriteWins,
    )?;

    for operation in merge_outcome.operations {
        if !crate::agent::hooks::planner_transform_operation_allowed(
            hook_point,
            &operation.operation,
        ) {
            return Err(format!(
                "hook `{}` attempted non-whitelisted planner patch `{}`",
                operation.hook_name, operation.operation.path
            ));
        }
        match operation.operation.path.as_str() {
            "provider_decision" => {
                decision = Some(parse_planner_provider_decision_patch(&operation.operation)?);
            }
            "provider_tool_call" => {
                let tool_call = parse_planner_tool_call_patch(&operation.operation)?;
                ensure_planner_decision(&mut decision).tool_call = Some(tool_call);
            }
            "selected_tool_call" => {
                selected_tool_call = Some(parse_planner_tool_call_patch(&operation.operation)?);
            }
            "selected_skill_id" => {
                let skill_id = parse_planner_string_patch(&operation.operation)?;
                selected_tool_call = Some(ToolCall {
                    call_id: None,
                    name: skill_id,
                    arguments: Value::Object(Map::new()),
                    plan: None,
                });
            }
            "decision_summary" => {}
            other => {
                return Err(format!("unsupported planner patch path `{other}`"));
            }
        }
    }

    Ok((decision, selected_tool_call))
}

fn ensure_planner_decision(decision: &mut Option<ProviderDecision>) -> &mut ProviderDecision {
    decision.get_or_insert_with(|| ProviderDecision {
        output_text: String::new(),
        tool_call: None,
        reasoning_content: None,
        reasoning_content_value: None,
        assistant_message: None,
        provider_source: "planner-hook".to_string(),
        provider_mode: "hook_transform".to_string(),
        fallback_reason: None,
        token_usage: None,
    })
}

fn parse_planner_provider_decision_patch(
    operation: &HookPatchOperation,
) -> Result<ProviderDecision, String> {
    let Some(value_text) = operation.value_text.as_deref() else {
        return Err("planner provider_decision patch requires value_text".to_string());
    };
    serde_json::from_str(value_text).map_err(|error| {
        format!(
            "invalid planner provider_decision patch payload for `{}`: {error}",
            operation.path
        )
    })
}

fn parse_planner_tool_call_patch(operation: &HookPatchOperation) -> Result<ToolCall, String> {
    let Some(value_text) = operation.value_text.as_deref() else {
        return Err(format!(
            "planner tool_call patch on `{}` requires value_text",
            operation.path
        ));
    };
    serde_json::from_str(value_text).map_err(|error| {
        format!(
            "invalid planner tool_call patch payload for `{}`: {error}",
            operation.path
        )
    })
}

fn parse_planner_string_patch(operation: &HookPatchOperation) -> Result<String, String> {
    let Some(value_text) = operation.value_text.as_deref() else {
        return Err(format!(
            "planner string patch on `{}` requires value_text",
            operation.path
        ));
    };
    serde_json::from_str(value_text).map_err(|error| {
        format!(
            "invalid planner string patch payload for `{}`: {error}",
            operation.path
        )
    })
}

fn apply_graph_decision_patches(
    decision: &mut GraphDecision,
    execution_results: &[crate::agent::hooks::HookExecutionResult],
) -> Result<(), String> {
    let merge_outcome = crate::agent::hooks::merge_patch_results(
        execution_results,
        HookPatchConflictPolicy::LastWriteWins,
    )?;

    for operation in merge_outcome.operations {
        if !crate::agent::hooks::planner_transform_operation_allowed(
            &PlannerHookPoint::GraphDecision,
            &operation.operation,
        ) {
            return Err(format!(
                "hook `{}` attempted non-whitelisted graph decision patch `{}`",
                operation.hook_name, operation.operation.path
            ));
        }
        match operation.operation.path.as_str() {
            "decision_summary" => {
                decision.summary = parse_planner_string_patch(&operation.operation)?;
            }
            other => {
                return Err(format!("unsupported graph decision patch path `{other}`"));
            }
        }
    }

    Ok(())
}

fn blocked_tool_result(tool_call: &ToolCall, error: &str) -> crate::agent::tools::ToolResult {
    crate::agent::tools::ToolResult {
        tool_name: tool_call.name.clone(),
        status: "error".to_string(),
        output: error.to_string(),
        duration_ms: 0,
    }
}

fn build_blocked_capability_invocation_record(
    tool_call: &ToolCall,
    error: &str,
) -> crate::agent::telemetry::CapabilityInvocationRecord {
    crate::agent::telemetry::CapabilityInvocationRecord {
        tool_name: tool_call.name.clone(),
        capability_id: None,
        source_id: None,
        source_kind: None,
        capability_kind: None,
        invocation_mode: None,
        failure_kind: Some("hook_blocked".to_string()),
        requires_approval: None,
        host_mediated: None,
        permission_scope: None,
        skill_id: None,
        skill_source_id: None,
        composed_capability_refs: None,
        composed_capability_kinds: None,
        failure_layer: Some(error.to_string()),
    }
}

fn build_blocked_skill_invocation_record(
    tool_call: &ToolCall,
    skill: Option<&crate::agent::capability_bridge::SkillDescriptor>,
    error: &str,
) -> crate::agent::telemetry::CapabilityInvocationRecord {
    crate::agent::telemetry::CapabilityInvocationRecord {
        tool_name: tool_call.name.clone(),
        capability_id: None,
        source_id: None,
        source_kind: None,
        capability_kind: None,
        invocation_mode: None,
        failure_kind: Some("hook_blocked".to_string()),
        requires_approval: None,
        host_mediated: None,
        permission_scope: None,
        skill_id: skill.map(|descriptor| descriptor.skill_id.clone()),
        skill_source_id: skill.map(|descriptor| descriptor.source_id.clone()),
        composed_capability_refs: skill
            .map(|descriptor| descriptor.composed_capability_refs.clone()),
        composed_capability_kinds: skill.map(|descriptor| {
            descriptor
                .composed_capability_kinds
                .iter()
                .map(|kind| kind.as_str().to_string())
                .collect()
        }),
        failure_layer: Some(error.to_string()),
    }
}

fn annotate_capability_tool_activities(
    mut activities: Vec<TurnToolActivity>,
    invocation_record: crate::agent::telemetry::CapabilityInvocationRecord,
) -> Vec<TurnToolActivity> {
    if let Some(parent) = activities.first_mut() {
        parent.capability_invocation = Some(invocation_record);
    }
    activities
}

fn build_skill_tool_result(
    tool_call: &ToolCall,
    execution: &SkillToolExecutionResult,
) -> crate::agent::tools::ToolResult {
    let status = if execution.failure_layer.is_none()
        && execution
            .capability_executions
            .iter()
            .all(|result| result.tool_result.status == "ok")
    {
        "ok"
    } else {
        "error"
    };
    let duration_ms = execution
        .capability_executions
        .iter()
        .map(|result| result.tool_result.duration_ms)
        .sum();
    let skill_label = execution
        .skill
        .as_ref()
        .map(|descriptor| descriptor.label.as_str())
        .unwrap_or(tool_call.name.as_str());
    let mut lines = vec![format!("skill `{skill_label}` execution summary:")];
    for result in &execution.capability_executions {
        lines.push(format!(
            "- [{}] {} -> {}",
            result.tool_result.status,
            result.tool_result.tool_name,
            preview_text(&result.tool_result.output, 120)
        ));
    }
    if let Some(layer) = execution.failure_layer.as_ref() {
        lines.push(format!("failure_layer={}", layer.as_str()));
    }

    crate::agent::tools::ToolResult {
        tool_name: tool_call.name.clone(),
        status: status.to_string(),
        output: lines.join("\n"),
        duration_ms,
    }
}
/*
    #[cfg(any())]
    fn start_turn_stream_uses_compat_sync_for_deepseek_tool_followup() {
        let final_text = "deepseek 工具 follow-up 已直接成功返回。";
        let server = MockHttpServer::start(vec![
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "先调用工具。",
                            "reasoning_content": "需要先读取目录再回答。",
                            "tool_calls": [
                                {
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": "{\"path\":\".\",\"limit\":40}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            })),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": final_text,
                            "reasoning_content": "工具结果已经足够，直接收口。"
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 60,
                    "completion_tokens": 24,
                    "total_tokens": 84
                }
            })),
        ]);
        let mut runtime =
            build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-deepseek-followup-compat".to_string(),
            TurnInput {
                message: "先列出文件再总结".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("deepseek-followup-compat".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let first_delta = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .expect("delta event");
        let text_delta = events
            .iter()
            .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .find(|payload| payload.text.is_some())
            .expect("text delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(requests.len(), 2);
        let decision_request: serde_json::Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        let followup_request: serde_json::Value =
            serde_json::from_str(&requests[1]).expect("followup request should be json");
        assert_eq!(
            decision_request.get("stream").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            followup_request.get("stream").and_then(Value::as_bool),
            Some(false)
        );
        assert!(followup_request.get("stream_options").is_none());
        assert_eq!(
            followup_request
                .get("messages")
                .and_then(Value::as_array)
                .and_then(|messages| messages.get(1))
                .and_then(|message| message.get("reasoning_content"))
                .and_then(Value::as_str),
            Some("需要先读取目录再回答。")
        );
        assert_eq!(first_delta.text.as_deref(), Some(final_text));
        assert_eq!(completed.phase.as_deref(), Some("completed"));
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_followup_stream_compat_sync")
        );
        assert_eq!(completed.fallback_reason, None);
    }

    #[cfg(any())]
    fn start_turn_stream_uses_live_stream_for_deepseek_tool_followup() {
        let final_text = "deepseek follow-up completed";
        let server = MockHttpServer::start(vec![
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "need workspace listing before answering"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "call a tool first",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_list_files",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_list_files",
                                            "arguments": "{\"path\":\".\",\"limit\":40}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 60,
                        "completion_tokens": 12,
                        "total_tokens": 72
                    }
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "tool output is sufficient"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 60,
                        "completion_tokens": 24,
                        "total_tokens": 84
                    }
                }),
            ]),
        ]);
        let mut runtime =
            build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-deepseek-followup-compat".to_string(),
            TurnInput {
                message: "read Cargo.toml then answer".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("deepseek-followup-compat".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let text_delta = events
            .iter()
            .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .find(|payload| payload.text.as_deref() == Some(final_text))
            .expect("text delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(requests.len(), 2);
        let decision_request: Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        let followup_request: Value =
            serde_json::from_str(&requests[1]).expect("followup request should be json");
        assert_eq!(
            decision_request.get("stream").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            followup_request.get("stream").and_then(Value::as_bool),
            Some(false)
        );
        assert!(followup_request.get("stream_options").is_none());
        let replayed_assistant_message = followup_request
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages.iter().find(|message| {
                    message.get("role").and_then(Value::as_str) == Some("assistant")
                        && message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(|calls| !calls.is_empty())
                            .unwrap_or(false)
                })
            })
            .expect("follow-up request should replay assistant tool call message");
        assert_eq!(
            replayed_assistant_message
                .get("reasoning_content")
                .and_then(Value::as_str),
            Some("need workspace listing before answering")
        );
        assert_eq!(first_delta.text.as_deref(), Some(final_text));
        assert_eq!(completed.phase.as_deref(), Some("completed"));
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_followup_stream_compat_sync")
        );
        assert_eq!(completed.fallback_reason, None);
    }
}

*/

fn planner_decision_can_override_native_tool_flow(decision: &ProviderDecision) -> bool {
    decision
        .tool_call
        .as_ref()
        .and_then(|call| call.plan.as_ref())
        .is_some()
}

fn graph_decision_kind_label(kind: &GraphDecisionKind) -> &'static str {
    match kind {
        GraphDecisionKind::Continue => "continue",
        GraphDecisionKind::WaitUser => "wait_user",
        GraphDecisionKind::Pause => "pause",
        GraphDecisionKind::Complete => "complete",
        GraphDecisionKind::Fail => "fail",
        GraphDecisionKind::Cancel => "cancel",
    }
}

fn validate_turn_images(images: &[TurnInputImage]) -> Result<(), String> {
    if images.len() > MAX_TURN_IMAGES {
        return Err(format!(
            "Too many images attached for a single turn. Limit={MAX_TURN_IMAGES}."
        ));
    }

    let total_bytes = images
        .iter()
        .map(TurnInputImage::payload_size_bytes)
        .sum::<u64>();
    if total_bytes > MAX_TURN_IMAGE_BYTES {
        return Err(format!(
            "Attached image payload is too large for a single turn. Limit={} bytes.",
            MAX_TURN_IMAGE_BYTES
        ));
    }

    Ok(())
}

fn should_recall_recent_images(retrieved: &RetrievedContextState) -> bool {
    let latest_user_message_has_attachments = retrieved
        .session_context
        .recent_history
        .iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| !message.attachments.is_empty())
        .unwrap_or(false);
    if !latest_user_message_has_attachments {
        return false;
    }

    retrieved.turn_context.references_image
}

fn recalled_image_limit(user_message: &str) -> usize {
    if user_message.contains("这几张")
        || user_message.contains("那几张")
        || user_message.contains("那组图")
        || user_message.contains("those images")
        || user_message.contains("these images")
    {
        MAX_TURN_IMAGES
    } else {
        1
    }
}

fn normalize_tool_directive(
    mut tool_call: ToolCall,
    assistant_message: Option<Value>,
    output_text: &str,
    reasoning_content: Option<&str>,
    reasoning_content_value: Option<&Value>,
) -> Result<NormalizedToolDirective, String> {
    if !tool_call.name.trim().is_empty() {
        return Ok(NormalizedToolDirective {
            tool_call,
            assistant_message,
        });
    }

    let repaired_name = infer_tool_name_from_arguments(&tool_call.arguments).ok_or_else(|| {
        format!(
            "provider 返回了缺少工具名的 tool call，且当前无法根据参数自动修复；arguments={}",
            preview_text(&tool_call.arguments.to_string(), 200)
        )
    })?;
    runtime_log(format!(
        "turn:tool-call-repaired repaired_name={} args={}",
        repaired_name, tool_call.arguments
    ));
    tool_call.name = repaired_name;

    let rebuilt_message = match reasoning_content_value {
        Some(raw_reasoning) => provider_native_assistant_tool_call_message_with_reasoning_value(
            non_empty_text(output_text),
            Some(raw_reasoning),
            &tool_call,
        ),
        None => provider_native_assistant_tool_call_message(
            non_empty_text(output_text),
            reasoning_content,
            &tool_call,
        ),
    };

    Ok(NormalizedToolDirective {
        assistant_message: Some(rebuilt_message),
        tool_call,
    })
}

fn infer_tool_name_from_arguments(arguments: &Value) -> Option<String> {
    let object = arguments.as_object()?;
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let has_query = object.contains_key("query");
    let has_limit = object.contains_key("limit");
    let has_line_count = object.contains_key("lineCount");
    let has_start_line = object.contains_key("startLine");

    if object
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
    {
        return Some("echo_input".to_string());
    }

    let path = path?;
    if has_start_line {
        return Some("workspace_read_file_segment".to_string());
    }
    if has_query || has_line_count {
        return Some("workspace_gather_context".to_string());
    }
    if has_limit {
        if looks_like_file_path(path) {
            return Some("workspace_read_file".to_string());
        }
        return Some("workspace_list_files".to_string());
    }
    if looks_like_file_path(path) {
        return Some("workspace_read_file".to_string());
    }

    Some("workspace_path_info".to_string())
}

fn looks_like_file_path(path: &str) -> bool {
    Path::new(path).extension().is_some()
}

fn non_empty_text(text: &str) -> Option<&str> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn native_transcript_for_completed_turn(
    user_message: &str,
    decision: &ProviderDecision,
    use_provider_native_tool_flow: bool,
) -> Option<Vec<Value>> {
    if !use_provider_native_tool_flow {
        return None;
    }

    let assistant_message = decision.assistant_message.clone().unwrap_or_else(|| {
        match decision.reasoning_content_value.as_ref() {
            Some(raw_reasoning) => provider_native_assistant_message_with_reasoning_value(
                &decision.output_text,
                Some(raw_reasoning),
            ),
            None => provider_native_assistant_message_with_reasoning(
                &decision.output_text,
                decision.reasoning_content.as_deref(),
            ),
        }
    });

    Some(vec![
        provider_native_user_message(user_message),
        assistant_message,
    ])
}

fn top_level_tool_activities(tool_activities: &[TurnToolActivity]) -> Vec<&TurnToolActivity> {
    tool_activities
        .iter()
        .filter(|activity| !activity.id.contains("-planned-") && !activity.id.contains("-child-"))
        .collect()
}

fn tool_activities_for_parent(
    tool_activities: &[TurnToolActivity],
    parent: &TurnToolActivity,
) -> Vec<TurnToolActivity> {
    let prefix = format!("{}-", parent.id);
    tool_activities
        .iter()
        .filter(|activity| activity.id == parent.id || activity.id.starts_with(&prefix))
        .cloned()
        .collect()
}

fn timeline_state_for_phase(phase: &str) -> String {
    match phase {
        "cancelled" => "cancelled".to_string(),
        "failed" => "error".to_string(),
        _ => "completed".to_string(),
    }
}

fn build_context_uses_retrieval(build_context_observation: &BuildContextObservation) -> bool {
    build_context_observation.message_count > 2
        || !build_context_observation.prefix_mutation_reasons.is_empty()
        || !build_context_observation
            .semi_stable_context_text
            .trim()
            .is_empty()
}

fn build_stream_started_trace_timeline(
    user_message: &str,
    provider_meta: &ProviderEventMeta,
    build_context_observation: &BuildContextObservation,
) -> Vec<TraceTimelineEntry> {
    let mut sequence = 1_u64;
    let mut timeline = Vec::new();

    timeline.push(TraceTimelineEntry {
        id: format!("input-{}", sequence),
        kind: "input".to_string(),
        label: "RECEIVE INPUT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: None,
        provider_name: None,
        provider_protocol: None,
        provider_model: None,
        provider_source: None,
        provider_mode: None,
        build_context_observation: None,
        tool_activities: Vec::new(),
        text: Some(user_message.to_string()),
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    if build_context_uses_retrieval(build_context_observation) {
        timeline.push(TraceTimelineEntry {
            id: format!("retrieval-{}", sequence),
            kind: "prepare_retrieval".to_string(),
            label: "PREPARE RETRIEVAL".to_string(),
            state: "completed".to_string(),
            sequence,
            provider_requested_name: Some(provider_meta.requested_name.clone()),
            provider_name: Some(provider_meta.provider_name.clone()),
            provider_protocol: Some(provider_meta.protocol.clone()),
            provider_model: Some(provider_meta.model.clone()),
            provider_source: None,
            provider_mode: None,
            build_context_observation: None,
            tool_activities: Vec::new(),
            text: None,
            reasoning_content: None,
            fallback_reason: None,
            error: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
        });
        sequence += 1;
    }

    timeline.push(TraceTimelineEntry {
        id: format!("context-{}", sequence),
        kind: "build_context".to_string(),
        label: "BUILD CONTEXT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: Some(provider_meta.requested_name.clone()),
        provider_name: Some(provider_meta.provider_name.clone()),
        provider_protocol: Some(provider_meta.protocol.clone()),
        provider_model: Some(provider_meta.model.clone()),
        provider_source: None,
        provider_mode: None,
        build_context_observation: Some(build_context_observation.clone()),
        tool_activities: Vec::new(),
        text: None,
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    timeline.push(TraceTimelineEntry {
        id: format!("model-{}", sequence),
        kind: "call_model".to_string(),
        label: "CALL MODEL #1".to_string(),
        state: "active".to_string(),
        sequence,
        provider_requested_name: Some(provider_meta.requested_name.clone()),
        provider_name: Some(provider_meta.provider_name.clone()),
        provider_protocol: Some(provider_meta.protocol.clone()),
        provider_model: Some(provider_meta.model.clone()),
        provider_source: None,
        provider_mode: None,
        build_context_observation: None,
        tool_activities: Vec::new(),
        text: None,
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });

    timeline
}

#[allow(clippy::too_many_arguments)]
fn build_stream_progress_trace_timeline(
    user_message: &str,
    provider_meta: &ProviderEventMeta,
    provider_source: Option<&str>,
    provider_mode: Option<&str>,
    build_context_observation: &BuildContextObservation,
    tool_activities: &[TurnToolActivity],
    model_output_text: Option<&str>,
    model_reasoning_content: Option<&str>,
    first_token_latency_ms: Option<u64>,
    phase: &str,
) -> Vec<TraceTimelineEntry> {
    let top_level_tools = top_level_tool_activities(tool_activities);
    let model_hops = if phase == "calling_tool" {
        top_level_tools.len().max(1)
    } else {
        top_level_tools.len() + 1
    };
    let mut sequence = 1_u64;
    let mut timeline = Vec::new();

    timeline.push(TraceTimelineEntry {
        id: format!("input-{}", sequence),
        kind: "input".to_string(),
        label: "RECEIVE INPUT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: None,
        provider_name: None,
        provider_protocol: None,
        provider_model: None,
        provider_source: None,
        provider_mode: None,
        build_context_observation: None,
        tool_activities: Vec::new(),
        text: Some(user_message.to_string()),
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    if build_context_uses_retrieval(build_context_observation) {
        timeline.push(TraceTimelineEntry {
            id: format!("retrieval-{}", sequence),
            kind: "prepare_retrieval".to_string(),
            label: "PREPARE RETRIEVAL".to_string(),
            state: "completed".to_string(),
            sequence,
            provider_requested_name: Some(provider_meta.requested_name.clone()),
            provider_name: Some(provider_meta.provider_name.clone()),
            provider_protocol: Some(provider_meta.protocol.clone()),
            provider_model: Some(provider_meta.model.clone()),
            provider_source: provider_source.map(str::to_string),
            provider_mode: provider_mode.map(str::to_string),
            build_context_observation: None,
            tool_activities: Vec::new(),
            text: None,
            reasoning_content: None,
            fallback_reason: None,
            error: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
        });
        sequence += 1;
    }

    timeline.push(TraceTimelineEntry {
        id: format!("context-{}", sequence),
        kind: "build_context".to_string(),
        label: "BUILD CONTEXT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: Some(provider_meta.requested_name.clone()),
        provider_name: Some(provider_meta.provider_name.clone()),
        provider_protocol: Some(provider_meta.protocol.clone()),
        provider_model: Some(provider_meta.model.clone()),
        provider_source: provider_source.map(str::to_string),
        provider_mode: provider_mode.map(str::to_string),
        build_context_observation: Some(build_context_observation.clone()),
        tool_activities: Vec::new(),
        text: None,
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    for model_index in 0..model_hops {
        let is_last_model = model_index + 1 == model_hops;
        let model_state = if phase == "calling_model" && is_last_model {
            "active"
        } else {
            "completed"
        };
        timeline.push(TraceTimelineEntry {
            id: format!("model-{}", sequence),
            kind: "call_model".to_string(),
            label: format!("CALL MODEL #{}", model_index + 1),
            state: model_state.to_string(),
            sequence,
            provider_requested_name: Some(provider_meta.requested_name.clone()),
            provider_name: Some(provider_meta.provider_name.clone()),
            provider_protocol: Some(provider_meta.protocol.clone()),
            provider_model: Some(provider_meta.model.clone()),
            provider_source: provider_source.map(str::to_string),
            provider_mode: provider_mode.map(str::to_string),
            build_context_observation: None,
            tool_activities: Vec::new(),
            text: if phase == "calling_model" && is_last_model {
                model_output_text.map(str::to_string)
            } else {
                None
            },
            reasoning_content: if phase == "calling_model" && is_last_model {
                model_reasoning_content.map(str::to_string)
            } else {
                None
            },
            fallback_reason: None,
            error: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: if phase == "calling_model" && is_last_model {
                first_token_latency_ms
            } else {
                None
            },
            turn_duration_ms: None,
        });
        sequence += 1;

        if let Some(parent_tool) = top_level_tools.get(model_index) {
            let grouped_tool_activities = tool_activities_for_parent(tool_activities, parent_tool);
            let tool_state = if parent_tool.status == "running" {
                "active"
            } else if parent_tool.status == "error" {
                "error"
            } else {
                "completed"
            };
            timeline.push(TraceTimelineEntry {
                id: format!("tool-{}", sequence),
                kind: "call_tool".to_string(),
                label: format!("CALL TOOL #{} · {}", model_index + 1, parent_tool.name),
                state: tool_state.to_string(),
                sequence,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                tool_activities: grouped_tool_activities,
                text: Some(parent_tool.summary.clone()),
                reasoning_content: None,
                fallback_reason: None,
                error: if parent_tool.status == "error" {
                    Some(parent_tool.summary.clone())
                } else {
                    None
                },
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
            });
            sequence += 1;
        }
    }

    timeline
}

#[allow(clippy::too_many_arguments)]
fn build_persisted_trace_timeline(
    user_message: &str,
    phase: &str,
    provider_meta: Option<&ProviderEventMeta>,
    provider_source: Option<&str>,
    provider_mode: Option<&str>,
    build_context_observation: Option<&BuildContextObservation>,
    tool_activities: &[TurnToolActivity],
    return_text: Option<&str>,
    return_reasoning_content: Option<&str>,
    fallback_reason: Option<&str>,
    error: Option<&str>,
    input_tokens: Option<u64>,
    cache_hit_input_tokens: Option<u64>,
    reasoning_tokens: Option<u64>,
    output_tokens: Option<u64>,
    total_tokens: Option<u64>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
) -> Vec<TraceTimelineEntry> {
    let terminal_state = timeline_state_for_phase(phase);
    let tool_hops = top_level_tool_activities(tool_activities);
    let mut sequence = 1_u64;
    let mut timeline = Vec::new();

    timeline.push(TraceTimelineEntry {
        id: format!("input-{}", sequence),
        kind: "input".to_string(),
        label: "RECEIVE INPUT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: None,
        provider_name: None,
        provider_protocol: None,
        provider_model: None,
        provider_source: None,
        provider_mode: None,
        build_context_observation: None,
        tool_activities: Vec::new(),
        text: Some(user_message.to_string()),
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    if let Some(observation) = build_context_observation {
        if build_context_uses_retrieval(observation) {
            timeline.push(TraceTimelineEntry {
                id: format!("retrieval-{}", sequence),
                kind: "prepare_retrieval".to_string(),
                label: "PREPARE RETRIEVAL".to_string(),
                state: "completed".to_string(),
                sequence,
                provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
                provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
                provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
                provider_model: provider_meta.map(|meta| meta.model.clone()),
                provider_source: provider_source.map(str::to_string),
                provider_mode: provider_mode.map(str::to_string),
                build_context_observation: None,
                tool_activities: Vec::new(),
                text: None,
                reasoning_content: None,
                fallback_reason: None,
                error: None,
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
            });
            sequence += 1;
        }
    }

    timeline.push(TraceTimelineEntry {
        id: format!("context-{}", sequence),
        kind: "build_context".to_string(),
        label: "BUILD CONTEXT".to_string(),
        state: "completed".to_string(),
        sequence,
        provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
        provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
        provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
        provider_model: provider_meta.map(|meta| meta.model.clone()),
        provider_source: provider_source.map(str::to_string),
        provider_mode: provider_mode.map(str::to_string),
        build_context_observation: build_context_observation.cloned(),
        tool_activities: Vec::new(),
        text: None,
        reasoning_content: None,
        fallback_reason: None,
        error: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
    });
    sequence += 1;

    let model_hops = if tool_hops.is_empty() {
        1
    } else {
        tool_hops.len() + 1
    };

    for model_index in 0..model_hops {
        let state = if phase == "failed" && model_index + 1 == model_hops {
            "error".to_string()
        } else if phase == "cancelled" && model_index + 1 == model_hops {
            "cancelled".to_string()
        } else {
            "completed".to_string()
        };
        timeline.push(TraceTimelineEntry {
            id: format!("model-{}", sequence),
            kind: "call_model".to_string(),
            label: format!("CALL MODEL #{}", model_index + 1),
            state,
            sequence,
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source: provider_source.map(str::to_string),
            provider_mode: provider_mode.map(str::to_string),
            build_context_observation: None,
            tool_activities: Vec::new(),
            text: None,
            reasoning_content: None,
            fallback_reason: None,
            error: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: if model_index == 0 {
                first_token_latency_ms
            } else {
                None
            },
            turn_duration_ms: None,
        });
        sequence += 1;

        if let Some(parent_tool) = tool_hops.get(model_index) {
            let grouped_tool_activities = tool_activities_for_parent(tool_activities, parent_tool);
            let tool_state = if parent_tool.status == "error" {
                "error".to_string()
            } else {
                "completed".to_string()
            };
            timeline.push(TraceTimelineEntry {
                id: format!("tool-{}", sequence),
                kind: "call_tool".to_string(),
                label: format!("CALL TOOL #{} · {}", model_index + 1, parent_tool.name),
                state: tool_state,
                sequence,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                tool_activities: grouped_tool_activities,
                text: Some(parent_tool.summary.clone()),
                reasoning_content: None,
                fallback_reason: None,
                error: if parent_tool.status == "error" {
                    Some(parent_tool.summary.clone())
                } else {
                    None
                },
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
            });
            sequence += 1;
        }
    }

    if let Some(last_model_entry) = timeline
        .iter_mut()
        .rev()
        .find(|entry| entry.kind == "call_model")
    {
        last_model_entry.state = terminal_state.to_string();
        last_model_entry.provider_requested_name =
            provider_meta.map(|meta| meta.requested_name.clone());
        last_model_entry.provider_name = provider_meta.map(|meta| meta.provider_name.clone());
        last_model_entry.provider_protocol = provider_meta.map(|meta| meta.protocol.clone());
        last_model_entry.provider_model = provider_meta.map(|meta| meta.model.clone());
        last_model_entry.provider_source = provider_source.map(str::to_string);
        last_model_entry.provider_mode = provider_mode.map(str::to_string);
        last_model_entry.text = return_text.map(str::to_string);
        last_model_entry.reasoning_content = return_reasoning_content.map(str::to_string);
        last_model_entry.fallback_reason = fallback_reason.map(str::to_string);
        last_model_entry.error = error.map(str::to_string);
        last_model_entry.input_tokens = input_tokens;
        last_model_entry.cache_hit_input_tokens = cache_hit_input_tokens;
        last_model_entry.reasoning_tokens = reasoning_tokens;
        last_model_entry.output_tokens = output_tokens;
        last_model_entry.total_tokens = total_tokens;
        last_model_entry.first_token_latency_ms = first_token_latency_ms;
        last_model_entry.turn_duration_ms = turn_duration_ms;
    }

    if phase == "completed" {
        timeline.push(TraceTimelineEntry {
            id: format!("checkpoint-{}", sequence),
            kind: "checkpoint_persist".to_string(),
            label: "PERSIST CHECKPOINT".to_string(),
            state: "completed".to_string(),
            sequence,
            provider_requested_name: provider_meta.map(|meta| meta.requested_name.clone()),
            provider_name: provider_meta.map(|meta| meta.provider_name.clone()),
            provider_protocol: provider_meta.map(|meta| meta.protocol.clone()),
            provider_model: provider_meta.map(|meta| meta.model.clone()),
            provider_source: provider_source.map(str::to_string),
            provider_mode: provider_mode.map(str::to_string),
            build_context_observation: None,
            tool_activities: Vec::new(),
            text: None,
            reasoning_content: None,
            fallback_reason: fallback_reason.map(str::to_string),
            error: None,
            input_tokens,
            cache_hit_input_tokens,
            reasoning_tokens,
            output_tokens,
            total_tokens,
            first_token_latency_ms,
            turn_duration_ms,
        });
    }

    timeline
}

fn build_turn_trace_title(message: &str) -> String {
    let compact = message.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return "空白输入".to_string();
    }

    let count = compact.chars().count();
    if count <= 44 {
        compact
    } else {
        format!("{}…", compact.chars().take(44).collect::<String>())
    }
}

fn recover_tool_followup_completion<P: crate::agent::provider::ProviderClient>(
    provider: &P,
    planning_request: &ProviderRequest,
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
    blocked_tool_call: &ToolCall,
    blocked_assistant_message: Option<&Value>,
    recovery_reason: &str,
    context_observation: &BuildContextObservation,
) -> RecoveredToolFollowup {
    let recovery_request =
        build_tool_followup_recovery_request(planning_request, user_message, hop_records);
    let synthetic_tool_result =
        build_tool_followup_recovery_tool_result(blocked_tool_call, hop_records, recovery_reason);
    let started_at = Instant::now();
    let mut response = provider_followup(
        provider,
        &recovery_request,
        &[],
        blocked_assistant_message,
        blocked_tool_call,
        &synthetic_tool_result,
    )
    .unwrap_or_else(|error| {
        build_local_tool_followup_recovery_response(&synthetic_tool_result, hop_records, &error)
    });
    let duration_ms = started_at.elapsed().as_millis() as u64;
    let provider_source_snapshot = response.provider_source.clone();
    let provider_mode_snapshot = response.provider_mode.clone();
    let token_usage_snapshot = response.token_usage.clone();
    let latency_kind = if response.provider_source == "provider_followup_sync" {
        ProviderLatencyKind::BufferedResponse
    } else {
        ProviderLatencyKind::Unknown
    };

    if response.tool_call.is_some() {
        response.fallback_reason = merge_fallback_reason(
            response.fallback_reason.clone(),
            Some("tool_followup_recovery_dropped_redundant_tool_call".to_string()),
        );
        response.tool_call = None;
        response.assistant_message = Some(match response.reasoning_content_value.as_ref() {
            Some(reasoning_value) => provider_native_assistant_message_with_reasoning_value(
                &response.output_text,
                Some(reasoning_value),
            ),
            None => provider_native_assistant_message_with_reasoning(
                &response.output_text,
                response.reasoning_content.as_deref(),
            ),
        });
    }

    RecoveredToolFollowup {
        response,
        provider_call_record: build_provider_call_cache_record(
            ProviderRequestKind::ToolFollowup,
            Some(provider_source_snapshot.as_str()),
            Some(provider_mode_snapshot.as_str()),
            token_usage_snapshot.as_ref(),
            None,
            Some(duration_ms),
            latency_kind,
            Some(context_observation),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
fn recover_tool_followup_completion_stream<P: crate::agent::provider::ProviderClient>(
    sink: &impl TurnEventSink,
    provider: &P,
    planning_request: &ProviderRequest,
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
    blocked_tool_call: &ToolCall,
    blocked_assistant_message: Option<&Value>,
    recovery_reason: &str,
    context_observation: &BuildContextObservation,
    turn_id: &str,
    session_id: Option<String>,
    first_token_latency: &Rc<Cell<Option<u64>>>,
    turn_started_at: &Instant,
) -> RecoveredToolFollowup {
    let recovery_request =
        build_tool_followup_recovery_request(planning_request, user_message, hop_records);
    let synthetic_tool_result =
        build_tool_followup_recovery_tool_result(blocked_tool_call, hop_records, recovery_reason);
    let started_at = Instant::now();
    let started_at_for_emit = started_at;
    let call_first_token_latency = Rc::new(Cell::new(None));
    let call_first_token_latency_for_emit = Rc::clone(&call_first_token_latency);
    let first_token_latency_for_emit = Rc::clone(first_token_latency);
    let reasoning_batcher = Rc::new(RefCell::new(StreamReasoningBatcher::default()));
    let reasoning_batcher_for_emit = Rc::clone(&reasoning_batcher);
    let turn_id_for_emit = turn_id.to_string();
    let session_id_for_emit = session_id.clone();
    let turn_started_at_for_latency = *turn_started_at;
    let emitted_text_ref = Rc::new(RefCell::new(String::new()));
    let emitted_reasoning_chars_ref = Rc::new(Cell::new(0usize));
    let emitted_text_for_emit = Rc::clone(&emitted_text_ref);
    let emitted_reasoning_chars_for_emit = Rc::clone(&emitted_reasoning_chars_ref);

    let mut response = provider_followup_stream(
        provider,
        &recovery_request,
        &[],
        blocked_assistant_message,
        blocked_tool_call,
        &synthetic_tool_result,
        move |delta| {
            if call_first_token_latency_for_emit.get().is_none() {
                let value = started_at_for_emit.elapsed().as_millis() as u64;
                call_first_token_latency_for_emit.set(Some(value));
            }
            let latency = if first_token_latency_for_emit.get().is_none() {
                let value = turn_started_at_for_latency.elapsed().as_millis() as u64;
                first_token_latency_for_emit.set(Some(value));
                Some(value)
            } else {
                None
            };
            match delta {
                ProviderStreamChunk::Text(text) => {
                    if let Some(reasoning) = reasoning_batcher_for_emit.borrow_mut().flush() {
                        emitted_reasoning_chars_for_emit.set(
                            emitted_reasoning_chars_for_emit
                                .get()
                                .saturating_add(reasoning.chars().count()),
                        );
                        emit_lightweight_delta(
                            sink,
                            &turn_id_for_emit,
                            None,
                            Some(reasoning),
                            latency,
                            session_id_for_emit.clone(),
                        );
                    }
                    emitted_text_for_emit.borrow_mut().push_str(&text);
                    emit_lightweight_delta(
                        sink,
                        &turn_id_for_emit,
                        Some(text),
                        None,
                        latency,
                        session_id_for_emit.clone(),
                    );
                }
                ProviderStreamChunk::Reasoning(reasoning) => {
                    if let Some(buffered_reasoning) =
                        reasoning_batcher_for_emit.borrow_mut().push(reasoning)
                    {
                        emitted_reasoning_chars_for_emit.set(
                            emitted_reasoning_chars_for_emit
                                .get()
                                .saturating_add(buffered_reasoning.chars().count()),
                        );
                        emit_lightweight_delta(
                            sink,
                            &turn_id_for_emit,
                            None,
                            Some(buffered_reasoning),
                            latency,
                            session_id_for_emit.clone(),
                        );
                    }
                }
            }
        },
    )
    .unwrap_or_else(|error| {
        build_local_tool_followup_recovery_response(&synthetic_tool_result, hop_records, &error)
    });

    if let Some(buffered_reasoning) = reasoning_batcher.borrow_mut().flush() {
        emitted_reasoning_chars_ref.set(
            emitted_reasoning_chars_ref
                .get()
                .saturating_add(buffered_reasoning.chars().count()),
        );
        emit_lightweight_delta(
            sink,
            turn_id,
            None,
            Some(buffered_reasoning),
            None,
            session_id.clone(),
        );
    }

    let emitted_text = emitted_text_ref.borrow().clone();
    let emitted_reasoning_chars = emitted_reasoning_chars_ref.get();
    if emitted_reasoning_chars == 0 {
        if let Some(reasoning_content) = response.reasoning_content.clone() {
            emit_lightweight_delta(
                sink,
                turn_id,
                None,
                Some(reasoning_content),
                None,
                session_id.clone(),
            );
        }
    }
    if !response.output_text.is_empty() {
        let missing_text = if emitted_text.is_empty() {
            Some(response.output_text.clone())
        } else {
            response
                .output_text
                .strip_prefix(&emitted_text)
                .filter(|suffix| !suffix.is_empty())
                .map(str::to_string)
        };
        if let Some(text) = missing_text {
            emit_lightweight_delta(sink, turn_id, Some(text), None, None, session_id.clone());
        }
    }

    let duration_ms = started_at.elapsed().as_millis() as u64;
    let provider_source_snapshot = response.provider_source.clone();
    let provider_mode_snapshot = response.provider_mode.clone();
    let token_usage_snapshot = response.token_usage.clone();
    let latency_kind = match response.provider_source.as_str() {
        "provider_followup_stream" => ProviderLatencyKind::ProviderStream,
        "provider_followup_sync" | "provider_followup_stream_sync_fallback" => {
            ProviderLatencyKind::BufferedResponse
        }
        _ => ProviderLatencyKind::Unknown,
    };

    if response.tool_call.is_some() {
        response.fallback_reason = merge_fallback_reason(
            response.fallback_reason.clone(),
            Some("tool_followup_recovery_dropped_redundant_tool_call".to_string()),
        );
        response.tool_call = None;
        response.assistant_message = Some(match response.reasoning_content_value.as_ref() {
            Some(reasoning_value) => provider_native_assistant_message_with_reasoning_value(
                &response.output_text,
                Some(reasoning_value),
            ),
            None => provider_native_assistant_message_with_reasoning(
                &response.output_text,
                response.reasoning_content.as_deref(),
            ),
        });
    }

    RecoveredToolFollowup {
        response,
        provider_call_record: build_provider_call_cache_record(
            ProviderRequestKind::ToolFollowup,
            Some(provider_source_snapshot.as_str()),
            Some(provider_mode_snapshot.as_str()),
            token_usage_snapshot.as_ref(),
            if latency_kind == ProviderLatencyKind::ProviderStream {
                call_first_token_latency.get()
            } else {
                None
            },
            Some(duration_ms),
            latency_kind,
            Some(context_observation),
        ),
    }
}

fn emit_lightweight_delta(
    sink: &impl TurnEventSink,
    turn_id: &str,
    text: Option<String>,
    reasoning_content: Option<String>,
    first_token_latency_ms: Option<u64>,
    session_id: Option<String>,
) {
    emit_stream_event(
        sink,
        "turn:delta",
        turn_id.to_string(),
        "delta",
        Some("calling_model"),
        text,
        reasoning_content,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        first_token_latency_ms,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        session_id,
    );
}

fn build_tool_followup_recovery_request(
    planning_request: &ProviderRequest,
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
) -> ProviderRequest {
    let mut request = planning_request.clone();
    request.native_messages = tool_turn_native_transcript_prefix(user_message, hop_records);
    request.observation = Default::default();
    request
}

fn tool_turn_native_transcript_prefix(
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
) -> Vec<Value> {
    let mut transcript = vec![provider_native_user_message(user_message)];
    for hop in hop_records {
        transcript.push(tool_request_assistant_message(hop));
        transcript.push(provider_native_tool_result_message(
            &hop.tool_call,
            &hop.tool_result,
        ));
    }
    transcript
}

fn build_tool_followup_recovery_tool_result(
    blocked_tool_call: &ToolCall,
    hop_records: &[ToolTurnHopRecord],
    recovery_reason: &str,
) -> crate::agent::tools::ToolResult {
    let recent_results = hop_records
        .iter()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|hop| {
            json!({
                "tool": hop.tool_call.name,
                "summary": local_tool_result_summary(&hop.tool_result),
                "output_preview": preview_text(&hop.tool_result.output, 400)
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({
        "status": "blocked_redundant_followup",
        "reason": recovery_reason,
        "instruction": "请不要继续申请工具；请基于当前上下文直接给出最佳答案。如果信息仍不完整，请明确缺口。",
        "recent_tool_results": recent_results
    });

    crate::agent::tools::ToolResult {
        tool_name: blocked_tool_call.name.clone(),
        status: "blocked".to_string(),
        output: serde_json::to_string_pretty(&payload)
            .unwrap_or_else(|_| recovery_reason.to_string()),
        duration_ms: 0,
    }
}

fn build_local_tool_followup_recovery_response(
    synthetic_tool_result: &crate::agent::tools::ToolResult,
    hop_records: &[ToolTurnHopRecord],
    error: &str,
) -> ProviderResponse {
    let recent_summaries = hop_records
        .iter()
        .rev()
        .take(3)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(|hop| {
            format!(
                "- {}: {}",
                hop.tool_call.name,
                local_tool_result_summary(&hop.tool_result)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let output_text = format!(
        "已停止重复探索并进入本地收口。\n原因：{}\n\n最近已拿到的上下文：\n{}\n\n如需更精确答案，请缩小到更具体的文件或符号。",
        preview_text(error, 240),
        if recent_summaries.is_empty() {
            "- 暂无可复用的工具结果。".to_string()
        } else {
            recent_summaries
        }
    );

    ProviderResponse {
        output_text: output_text.clone(),
        tool_call: None,
        reasoning_content: None,
        reasoning_content_value: None,
        assistant_message: Some(provider_native_assistant_message_with_reasoning(
            &output_text,
            Some(&synthetic_tool_result.output),
        )),
        provider_source: "provider_followup_recovery_local_fallback".to_string(),
        provider_mode: "fallback".to_string(),
        fallback_reason: Some(format!(
            "tool_followup_recovery_failed:{}",
            preview_text(error, 180)
        )),
        token_usage: Some(TokenUsage {
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
        }),
    }
}

fn local_tool_result_summary(tool_result: &crate::agent::tools::ToolResult) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(&tool_result.output) {
        if let Some(summary_text) = value
            .get("summary")
            .and_then(|summary| summary.get("text"))
            .and_then(Value::as_str)
        {
            return summary_text.to_string();
        }

        if let Some(plan_summary) = value
            .get("plan")
            .and_then(|plan| plan.get("summary"))
            .and_then(Value::as_str)
        {
            return plan_summary.to_string();
        }

        return preview_text(&value.to_string(), 240);
    }

    preview_text(&tool_result.output, 240)
}

fn native_transcript_for_tool_turn(
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
    final_response: &ProviderResponse,
) -> Option<Vec<Value>> {
    let mut transcript = vec![provider_native_user_message(user_message)];
    for hop in hop_records {
        transcript.push(tool_request_assistant_message(hop));
        transcript.push(provider_native_tool_result_message(
            &hop.tool_call,
            &hop.tool_result,
        ));
    }
    transcript.push(final_assistant_message(final_response));
    Some(transcript)
}

fn tool_request_assistant_message(hop: &ToolTurnHopRecord) -> Value {
    match hop.assistant_reasoning_content_value.as_ref() {
        Some(raw_reasoning) => provider_native_assistant_tool_call_message_with_reasoning_value(
            text_if_present(&hop.assistant_output_text),
            Some(raw_reasoning),
            &hop.tool_call,
        ),
        None => provider_native_assistant_tool_call_message(
            text_if_present(&hop.assistant_output_text),
            hop.assistant_reasoning_content.as_deref(),
            &hop.tool_call,
        ),
    }
}

fn final_assistant_message(response: &ProviderResponse) -> Value {
    response.assistant_message.clone().unwrap_or_else(|| {
        match response.reasoning_content_value.as_ref() {
            Some(raw_reasoning) => provider_native_assistant_message_with_reasoning_value(
                &response.output_text,
                Some(raw_reasoning),
            ),
            None => provider_native_assistant_message_with_reasoning(
                &response.output_text,
                response.reasoning_content.as_deref(),
            ),
        }
    })
}

fn text_if_present(text: &str) -> Option<&str> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}

fn merge_fallback_reason(existing: Option<String>, next: Option<String>) -> Option<String> {
    match (existing, next) {
        (Some(existing), Some(next)) if !next.trim().is_empty() && existing != next => {
            Some(format!("{} | {}", existing, next))
        }
        (Some(existing), _) => Some(existing),
        (None, Some(next)) if !next.trim().is_empty() => Some(next),
        (None, _) => None,
    }
}

fn build_provider_call_cache_record(
    request_kind: ProviderRequestKind,
    provider_source: Option<&str>,
    provider_mode: Option<&str>,
    token_usage: Option<&TokenUsage>,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
    latency_kind: ProviderLatencyKind,
    build_context_observation: Option<&BuildContextObservation>,
) -> ProviderCallCacheRecord {
    let (input_tokens, cache_hit_input_tokens, reasoning_tokens, output_tokens, total_tokens) =
        token_usage_parts(token_usage);

    ProviderCallCacheRecord {
        request_kind,
        provider_source: provider_source.map(str::to_string),
        provider_mode: provider_mode.map(str::to_string),
        input_tokens,
        cache_hit_input_tokens,
        cache_miss_input_tokens: derive_cache_miss_input_tokens(token_usage),
        reasoning_tokens,
        output_tokens,
        total_tokens,
        first_token_latency_ms,
        turn_duration_ms,
        latency_kind,
        prefix_mutation_reasons: build_context_observation
            .map(|observation| observation.prefix_mutation_reasons.clone())
            .unwrap_or_default(),
    }
}

fn derive_cache_miss_input_tokens(token_usage: Option<&TokenUsage>) -> Option<u64> {
    let usage = token_usage?;
    let input_tokens = usage.input_tokens?;
    let cache_hit_input_tokens = usage.cache_hit_input_tokens?;
    Some(input_tokens.saturating_sub(cache_hit_input_tokens))
}

fn merge_token_usage(
    existing: Option<TokenUsage>,
    next: Option<&TokenUsage>,
) -> Option<TokenUsage> {
    match (existing, next) {
        (Some(existing), Some(next)) => Some(TokenUsage {
            input_tokens: add_optional_u64(existing.input_tokens, next.input_tokens),
            cache_hit_input_tokens: add_optional_u64(
                existing.cache_hit_input_tokens,
                next.cache_hit_input_tokens,
            ),
            reasoning_tokens: add_optional_u64(existing.reasoning_tokens, next.reasoning_tokens),
            output_tokens: add_optional_u64(existing.output_tokens, next.output_tokens),
            total_tokens: add_optional_u64(existing.total_tokens, next.total_tokens),
        }),
        (Some(existing), None) => Some(existing),
        (None, Some(next)) => Some(next.clone()),
        (None, None) => None,
    }
}

fn add_optional_u64(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.saturating_add(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn running_tool_activities_with_history(
    completed: &[TurnToolActivity],
    running: Vec<TurnToolActivity>,
) -> Vec<TurnToolActivity> {
    let mut combined = completed.to_vec();
    combined.extend(running);
    combined
}

#[derive(Default)]
struct StreamReasoningBatcher {
    buffer: String,
}

impl StreamReasoningBatcher {
    fn push(&mut self, reasoning: String) -> Option<String> {
        self.buffer.push_str(&reasoning);
        if self.buffer.chars().count() >= STREAM_REASONING_BATCH_CHARS {
            return self.flush();
        }
        None
    }

    fn flush(&mut self) -> Option<String> {
        if self.buffer.is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.buffer))
    }
}

fn canonicalize_tool_argument_value(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(canonicalize_tool_argument_value)
                .collect::<Vec<_>>(),
        ),
        Value::Object(map) => {
            let mut normalized = Map::new();
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                if let Some(entry) = map.get(&key) {
                    normalized.insert(key, canonicalize_tool_argument_value(entry));
                }
            }
            Value::Object(normalized)
        }
        _ => value.clone(),
    }
}

fn tool_call_signature(tool_call: &ToolCall) -> String {
    let normalized = canonicalize_tool_argument_value(&tool_call.arguments);
    format!(
        "{}:{}",
        tool_call.name,
        serde_json::to_string(&normalized).unwrap_or_else(|_| "{}".to_string())
    )
}

fn build_tool_hop_limit_error(limit: usize) -> String {
    format!(
        "同一 turn 内连续工具调用超过 {} 次，已停止继续 follow-up 以避免进入无限循环；如属复杂任务，可提高 PONY_AGENT_MAX_TOOL_HOPS_PER_TURN。",
        limit
    )
}

fn build_tool_followup_limit_error(limit: usize) -> String {
    format!(
        "同一 turn 内 follow-up 轮次超过 {} 次，已停止继续 follow-up 以避免重复探索；如属复杂任务，可提高 PONY_AGENT_MAX_TOOL_FOLLOWUPS_PER_TURN。",
        limit
    )
}

fn build_duplicate_tool_call_error(tool_call: &ToolCall) -> String {
    format!(
        "工具 `{}` 在同一 turn 内重复调用了近似相同的参数，已停止继续 follow-up 以避免重复探索。",
        tool_call.name
    )
}

fn build_tool_execution_error(tool_name: &str, output: &str) -> String {
    format!(
        "工具 `{}` 执行失败：{}",
        tool_name,
        preview_text(output, 160)
    )
}

#[cfg(test)]
fn tool_hop_limit_override_registry() -> &'static AtomicUsize {
    static OVERRIDE: OnceLock<AtomicUsize> = OnceLock::new();
    OVERRIDE.get_or_init(|| AtomicUsize::new(0))
}

fn max_tool_hops_per_turn() -> usize {
    #[cfg(test)]
    {
        let override_limit = tool_hop_limit_override_registry().load(AtomicOrdering::SeqCst);
        if override_limit > 0 {
            return override_limit;
        }
    }

    static MAX_TOOL_HOPS: OnceLock<usize> = OnceLock::new();
    *MAX_TOOL_HOPS.get_or_init(|| {
        parse_max_tool_hops_per_turn(std::env::var(MAX_TOOL_HOPS_ENV).ok().as_deref())
    })
}

fn parse_max_tool_hops_per_turn(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1..=MAX_ALLOWED_TOOL_HOPS_PER_TURN).contains(value))
        .unwrap_or(DEFAULT_MAX_TOOL_HOPS_PER_TURN)
}

#[cfg(test)]
fn tool_followup_limit_override_registry() -> &'static AtomicUsize {
    static OVERRIDE: OnceLock<AtomicUsize> = OnceLock::new();
    OVERRIDE.get_or_init(|| AtomicUsize::new(0))
}

fn max_tool_followups_per_turn() -> usize {
    #[cfg(test)]
    {
        let override_limit = tool_followup_limit_override_registry().load(AtomicOrdering::SeqCst);
        if override_limit > 0 {
            return override_limit;
        }
    }

    static MAX_TOOL_FOLLOWUPS: OnceLock<usize> = OnceLock::new();
    *MAX_TOOL_FOLLOWUPS.get_or_init(|| {
        parse_max_tool_followups_per_turn(std::env::var(MAX_TOOL_FOLLOWUPS_ENV).ok().as_deref())
    })
}

fn parse_max_tool_followups_per_turn(raw: Option<&str>) -> usize {
    raw.and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| (1..=MAX_ALLOWED_TOOL_FOLLOWUPS_PER_TURN).contains(value))
        .unwrap_or(DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::config::{
        ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    };
    use crate::agent::context::{DefaultTurnContextBuilder, TurnContextBuilder};
    use crate::agent::hooks::{
        hook_point_matches_canonical_boundary, AgentHookDescriptor, AgentHookExecutor,
        CapabilityMediationEnvelope, CapabilityMediationHookPoint, HookClass, HookFailurePolicy,
        HookPatchOperation, HookPatchOperationKind, HookPatchTarget, HookRecoveryMode,
        HookReplayRequirements, HookResultKind, HookSideEffectPersistenceRequirements,
        HookStructuredResult, HookTraceRequirements, PlannerFactsEnvelope, PlannerHookPoint,
        TurnHookPoint,
    };
    use crate::agent::planner::TurnPlanner;
    use crate::agent::session::{
        FileSessionBackend, SessionSnapshot, SessionStore, TurnHistoryMessage,
    };
    use crate::agent::telemetry::DefaultTurnTelemetryBuilder;
    use serde_json::json;
    use std::cell::RefCell;
    use std::fs;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct ToolHopLimitOverrideGuard {
        previous: usize,
    }

    impl ToolHopLimitOverrideGuard {
        fn set(limit: usize) -> Self {
            let previous = tool_hop_limit_override_registry().swap(limit, AtomicOrdering::SeqCst);
            Self { previous }
        }
    }

    impl Drop for ToolHopLimitOverrideGuard {
        fn drop(&mut self) {
            tool_hop_limit_override_registry().store(self.previous, AtomicOrdering::SeqCst);
        }
    }

    struct RecordingTurnEventSink {
        events: RefCell<Vec<(String, TurnStreamEvent)>>,
    }

    impl RecordingTurnEventSink {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
            }
        }
    }

    impl TurnEventSink for RecordingTurnEventSink {
        fn emit(&self, name: &str, payload: TurnStreamEvent) {
            self.events.borrow_mut().push((name.to_string(), payload));
        }
    }

    fn assert_hook_boundary_alignment(
        payload: &TurnStreamEvent,
        hook_point: TurnHookPoint,
        expected_event_type: &str,
        expected_phase: &str,
    ) {
        assert_eq!(payload.event_type.as_deref(), Some(expected_event_type));
        assert_eq!(payload.phase.as_deref(), Some(expected_phase));
        assert!(hook_point_matches_canonical_boundary(
            &hook_point,
            expected_event_type,
            expected_phase
        ));
    }

    #[derive(Clone)]
    struct StaticResolver {
        selection: ResolvedProviderSelection,
    }

    impl ProviderSelectionResolver for StaticResolver {
        fn resolve_provider_selection(
            &self,
            _provider_id: Option<&str>,
            _model_id: Option<&str>,
        ) -> ResolvedProviderSelection {
            self.selection.clone()
        }
    }

    struct PassthroughPlanner;

    impl TurnPlanner for PassthroughPlanner {
        fn preflight_decision(
            &self,
            _user_message: &str,
            _history: &[TurnHistoryMessage],
            _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        ) -> Option<ProviderDecision> {
            None
        }

        fn select_tool_call(
            &self,
            _user_message: &str,
            _history: &[TurnHistoryMessage],
            _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
            provider_tool_call: Option<ToolCall>,
        ) -> Option<ToolCall> {
            provider_tool_call
        }
    }

    struct SlowPassthroughPlanner {
        delay_ms: u64,
    }

    impl TurnPlanner for SlowPassthroughPlanner {
        fn preflight_decision(
            &self,
            _user_message: &str,
            _history: &[TurnHistoryMessage],
            _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        ) -> Option<ProviderDecision> {
            std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
            None
        }

        fn select_tool_call(
            &self,
            _user_message: &str,
            _history: &[TurnHistoryMessage],
            _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
            provider_tool_call: Option<ToolCall>,
        ) -> Option<ToolCall> {
            provider_tool_call
        }
    }

    struct ForcedToolPlanner {
        tool_name: String,
        arguments: Value,
    }

    impl TurnPlanner for ForcedToolPlanner {
        fn preflight_decision(
            &self,
            _user_message: &str,
            _history: &[TurnHistoryMessage],
            _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
        ) -> Option<ProviderDecision> {
            Some(ProviderDecision {
                output_text: String::new(),
                tool_call: Some(ToolCall {
                    call_id: Some("forced-tool-call".to_string()),
                    name: self.tool_name.clone(),
                    arguments: self.arguments.clone(),
                    plan: Some(crate::agent::tools::ToolPlan {
                        kind: "forced".to_string(),
                        summary: format!("强制执行工具 `{}`。", self.tool_name),
                        parallel: false,
                        continue_on_error: false,
                        steps: Vec::new(),
                    }),
                }),
                reasoning_content: None,
                reasoning_content_value: None,
                assistant_message: None,
                provider_source: "planner_preflight".to_string(),
                provider_mode: "preflight".to_string(),
                fallback_reason: None,
                token_usage: None,
            })
        }

        fn select_tool_call(
            &self,
            _user_message: &str,
            _history: &[TurnHistoryMessage],
            _available_skills: &[crate::agent::capability_bridge::SkillDescriptor],
            provider_tool_call: Option<ToolCall>,
        ) -> Option<ToolCall> {
            provider_tool_call
        }
    }

    struct StubToolExecutor;

    impl crate::agent::tools::ToolExecutor for StubToolExecutor {
        fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
            let output = match call.name.as_str() {
                "workspace_list_files" => {
                    "{\"entries\":[\"Cargo.toml\",\"tauri.conf.json\",\"src/\"]}".to_string()
                }
                "workspace_read_file" => {
                    "{\n  \"productName\": \"Pony Agent\",\n  \"version\": \"0.1.0\"\n}".to_string()
                }
                other => format!("unsupported tool in test: {}", other),
            };

            crate::agent::tools::ToolResult {
                tool_name: call.name.clone(),
                status: "ok".to_string(),
                output,
                duration_ms: 1,
            }
        }
    }

    struct SlowToolExecutor {
        delay_ms: u64,
    }

    impl crate::agent::tools::ToolExecutor for SlowToolExecutor {
        fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
            std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
            StubToolExecutor.execute(call)
        }
    }

    struct RequestStopToolExecutor {
        control: Arc<ExecutionControlRegistry>,
        turn_id: String,
        delay_ms: u64,
    }

    impl crate::agent::tools::ToolExecutor for RequestStopToolExecutor {
        fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
            std::thread::sleep(std::time::Duration::from_millis(self.delay_ms));
            let _ = self.control.request_stop(&self.turn_id);
            StubToolExecutor.execute(call)
        }
    }

    struct ErrorToolExecutor;

    struct CountingToolExecutor {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl crate::agent::tools::ToolExecutor for CountingToolExecutor {
        fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            StubToolExecutor.execute(call)
        }
    }

    struct FailingHookExecutor;

    struct RecordingToolExecutor {
        calls: Arc<Mutex<Vec<ToolCall>>>,
    }

    impl crate::agent::tools::ToolExecutor for RecordingToolExecutor {
        fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
            self.calls.lock().unwrap().push(call.clone());
            crate::agent::tools::ToolResult {
                tool_name: call.name.clone(),
                status: "ok".to_string(),
                output: call.arguments.to_string(),
                duration_ms: 1,
            }
        }
    }

    struct TransformingCapabilityHookExecutor;
    struct TransformingPlannerHookExecutor;

    impl AgentHookExecutor for FailingHookExecutor {
        fn execute(
            &self,
            descriptor: &AgentHookDescriptor,
            hook_point: TurnHookPoint,
        ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
            Err(format!(
                "intentional hook failure for `{}` on `{:?}`",
                descriptor.name, hook_point
            ))
        }
    }

    impl AgentHookExecutor for TransformingCapabilityHookExecutor {
        fn execute(
            &self,
            descriptor: &AgentHookDescriptor,
            hook_point: TurnHookPoint,
        ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
            NoopHookExecutor.execute(descriptor, hook_point)
        }

        fn execute_capability_mediation(
            &self,
            descriptor: &AgentHookDescriptor,
            hook_point: CapabilityMediationHookPoint,
            envelope: &CapabilityMediationEnvelope,
        ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
            let operations = match hook_point {
                CapabilityMediationHookPoint::CapabilityResolve => vec![HookPatchOperation {
                    target: HookPatchTarget::CapabilityMediation,
                    path: "request.arguments".to_string(),
                    operation: HookPatchOperationKind::Merge,
                    value_summary: Some("rewrite capability arguments".to_string()),
                    value_text: Some("{\"path\":\"src-tauri\"}".to_string()),
                }],
                CapabilityMediationHookPoint::SkillToolActionsResolve => vec![HookPatchOperation {
                    target: HookPatchTarget::CapabilityMediation,
                    path: "request.arguments".to_string(),
                    operation: HookPatchOperationKind::Merge,
                    value_summary: Some("rewrite skill arguments".to_string()),
                    value_text: Some("{\"message\":\"patched by hook\"}".to_string()),
                }],
                CapabilityMediationHookPoint::McpSourceIngress
                | CapabilityMediationHookPoint::SkillSourceIngress => Vec::new(),
            };
            Ok(crate::agent::hooks::HookExecutionResult {
                hook_name: descriptor.name.clone(),
                hook_class: HookClass::Transform,
                hook_point: turn_hook_point_for_capability_mediation_hook_point(&hook_point),
                hook_order: 0,
                result_kind: HookResultKind::Patch,
                structured_result: HookStructuredResult::Patch { operations },
                blocked: false,
                elapsed_ms: 0,
                input_summary: Some(envelope.argument_summary.clone()),
                persistence_evidence_ref: None,
                trace_summary: format!("hook rewrote mediation arguments at {:?}", hook_point),
            })
        }
    }

    impl AgentHookExecutor for TransformingPlannerHookExecutor {
        fn execute(
            &self,
            descriptor: &AgentHookDescriptor,
            hook_point: TurnHookPoint,
        ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
            NoopHookExecutor.execute(descriptor, hook_point)
        }

        fn execute_planner(
            &self,
            descriptor: &AgentHookDescriptor,
            hook_point: PlannerHookPoint,
            envelope: &PlannerFactsEnvelope,
        ) -> Result<crate::agent::hooks::HookExecutionResult, String> {
            let operations = match hook_point {
                PlannerHookPoint::TurnPreflight => vec![HookPatchOperation {
                    target: HookPatchTarget::PlannerFacts,
                    path: "provider_tool_call".to_string(),
                    operation: HookPatchOperationKind::Set,
                    value_summary: Some("rewrite provider tool call".to_string()),
                    value_text: Some(
                        "{\"call_id\":null,\"name\":\"workspace_list_files\",\"arguments\":{\"path\":\"src-tauri\",\"limit\":5},\"plan\":{\"kind\":\"forced\",\"summary\":\"hook rewritten preflight tool\",\"parallel\":false,\"continue_on_error\":false,\"steps\":[]}}".to_string(),
                    ),
                }],
                PlannerHookPoint::ToolSelection => vec![HookPatchOperation {
                    target: HookPatchTarget::PlannerFacts,
                    path: "selected_tool_call".to_string(),
                    operation: HookPatchOperationKind::Set,
                    value_summary: Some("rewrite selected tool call".to_string()),
                    value_text: Some(
                        "{\"call_id\":null,\"name\":\"workspace_list_files\",\"arguments\":{\"path\":\"tests\",\"limit\":3},\"plan\":null}".to_string(),
                    ),
                }],
                PlannerHookPoint::GraphDecision => vec![HookPatchOperation {
                    target: HookPatchTarget::PlannerFacts,
                    path: "decision_summary".to_string(),
                    operation: HookPatchOperationKind::Set,
                    value_summary: Some("rewrite graph decision summary".to_string()),
                    value_text: Some("\"planner summary patched by hook\"".to_string()),
                }],
            };
            Ok(crate::agent::hooks::HookExecutionResult {
                hook_name: descriptor.name.clone(),
                hook_class: HookClass::Transform,
                hook_point: turn_hook_point_for_planner_hook_point(&hook_point),
                hook_order: 0,
                result_kind: HookResultKind::Patch,
                structured_result: HookStructuredResult::Patch { operations },
                blocked: false,
                elapsed_ms: 0,
                input_summary: envelope.user_message_summary.clone(),
                persistence_evidence_ref: None,
                trace_summary: format!("hook rewrote planner payload at {:?}", hook_point),
            })
        }
    }

    impl crate::agent::tools::ToolExecutor for ErrorToolExecutor {
        fn execute(&self, call: &ToolCall) -> crate::agent::tools::ToolResult {
            crate::agent::tools::ToolResult {
                tool_name: call.name.clone(),
                status: "error".to_string(),
                output: format!("tool {} failed in test", call.name),
                duration_ms: 1,
            }
        }
    }

    struct MockHttpResponse {
        content_type: &'static str,
        body: String,
    }

    struct MockHttpServer {
        base_url: String,
        requests: Arc<Mutex<Vec<String>>>,
        handle: Option<thread::JoinHandle<()>>,
    }

    impl MockHttpServer {
        fn start(responses: Vec<MockHttpResponse>) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind mock server");
            let address = listener.local_addr().expect("mock server addr");
            let requests = Arc::new(Mutex::new(Vec::new()));
            let requests_for_thread = Arc::clone(&requests);

            let handle = thread::spawn(move || {
                for response in responses {
                    let (mut stream, _) = listener.accept().expect("accept mock request");
                    let body = read_http_request_body(&mut stream);
                    requests_for_thread.lock().unwrap().push(body);
                    write_http_response(&mut stream, response);
                }
            });

            Self {
                base_url: format!("http://{}/v1", address),
                requests,
                handle: Some(handle),
            }
        }

        fn finish(mut self) -> Vec<String> {
            if let Some(handle) = self.handle.take() {
                handle.join().expect("join mock server");
            }
            self.requests.lock().unwrap().clone()
        }
    }

    fn read_http_request_body(stream: &mut TcpStream) -> String {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 4096];
        let mut header_end = None;
        let mut content_length = 0usize;

        loop {
            let read = stream.read(&mut chunk).expect("read mock request");
            if read == 0 {
                break;
            }
            buffer.extend_from_slice(&chunk[..read]);

            if header_end.is_none() {
                header_end = find_header_end(&buffer);
                if let Some(end) = header_end {
                    let headers = String::from_utf8_lossy(&buffer[..end]).to_string();
                    content_length = parse_content_length(&headers);
                }
            }

            if let Some(end) = header_end {
                if buffer.len() >= end + content_length {
                    break;
                }
            }
        }

        let Some(end) = header_end else {
            return String::new();
        };
        String::from_utf8_lossy(&buffer[end..end + content_length]).to_string()
    }

    fn find_header_end(buffer: &[u8]) -> Option<usize> {
        buffer
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|position| position + 4)
    }

    fn parse_content_length(headers: &str) -> usize {
        headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("Content-Length") {
                    value.trim().parse::<usize>().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }

    fn write_http_response(stream: &mut TcpStream, response: MockHttpResponse) {
        let payload = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.content_type,
            response.body.len(),
            response.body
        );
        stream
            .write_all(payload.as_bytes())
            .expect("write mock response");
        stream.flush().expect("flush mock response");
    }

    fn json_response(body: serde_json::Value) -> MockHttpResponse {
        MockHttpResponse {
            content_type: "application/json",
            body: body.to_string(),
        }
    }

    fn json_completion(text: &str) -> MockHttpResponse {
        json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": text
                    }
                }
            ]
        }))
    }

    fn sse_response(chunks: &[serde_json::Value]) -> MockHttpResponse {
        let mut body = String::new();
        for chunk in chunks {
            body.push_str("data: ");
            body.push_str(&chunk.to_string());
            body.push_str("\n\n");
        }
        body.push_str("data: [DONE]\n\n");

        MockHttpResponse {
            content_type: "text/event-stream",
            body,
        }
    }

    fn test_provider_selection(base_url: String) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "test-openai".to_string(),
            provider_name: "test-openai".to_string(),
            protocol: crate::agent::provider::ProviderProtocol::OpenAi,
            base_url,
            api_key_env_var: "TEST_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "gpt-5.4".to_string(),
            temperature: 0.2,
            max_output_tokens: 1024,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
            },
        }
    }

    fn deepseek_provider_selection(base_url: String) -> ResolvedProviderSelection {
        ResolvedProviderSelection {
            requested_name: "deepseek".to_string(),
            provider_name: "deepseek".to_string(),
            protocol: crate::agent::provider::ProviderProtocol::OpenAi,
            base_url,
            api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
            api_key: Some("test-key".to_string()),
            model: "deepseek-v4-flash".to_string(),
            temperature: 0.2,
            max_output_tokens: 1024,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                context_window_tokens: Some(128_000),
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: false,
                supports_reasoning: true,
            },
        }
    }

    fn build_runtime_for_test(selection: ResolvedProviderSelection) -> AgentRuntime {
        AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            Box::new(StubToolExecutor),
            Box::new(PassthroughPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        )
    }

    fn build_runtime_for_test_with_tool_executor(
        selection: ResolvedProviderSelection,
        tool_executor: Box<dyn ToolExecutor>,
    ) -> AgentRuntime {
        AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            tool_executor,
            Box::new(PassthroughPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        )
    }

    fn build_runtime_with_session_store(
        selection: ResolvedProviderSelection,
        sessions: SessionStore,
    ) -> AgentRuntime {
        AgentRuntime::with_dependencies(
            sessions,
            Box::new(StaticResolver { selection }),
            Box::new(StubToolExecutor),
            Box::new(PassthroughPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        )
    }

    fn observe_hook_descriptor(
        name: &str,
        priority: i32,
        hook_point: TurnHookPoint,
    ) -> AgentHookDescriptor {
        AgentHookDescriptor {
            contract_version: "agent-hooks-v1".to_string(),
            name: name.to_string(),
            class: HookClass::Observe,
            priority,
            timeout_ms: 1_000,
            allowed_hook_points: vec![hook_point],
            allowed_result_kinds: vec![HookResultKind::Observe],
            can_block: false,
            default_failure_policy: HookFailurePolicy::Ignore,
            allowed_failure_policies: vec![HookFailurePolicy::Ignore],
            default_recovery_mode: HookRecoveryMode::ReplayRequired,
            trace_requirements: HookTraceRequirements {
                include_name: true,
                include_hook_point: true,
                include_elapsed_ms: true,
                include_result_summary: true,
            },
            replay_requirements: HookReplayRequirements {
                include_hook_order: true,
                include_input_summary: true,
            },
            side_effect_persistence_requirements: HookSideEffectPersistenceRequirements {
                require_persistence_evidence: false,
                require_effect_summary: false,
            },
        }
    }

    fn transform_hook_descriptor(
        name: &str,
        priority: i32,
        hook_point: TurnHookPoint,
    ) -> AgentHookDescriptor {
        AgentHookDescriptor {
            contract_version: "agent-hooks-v1".to_string(),
            name: name.to_string(),
            class: HookClass::Transform,
            priority,
            timeout_ms: 1_000,
            allowed_hook_points: vec![hook_point],
            allowed_result_kinds: vec![HookResultKind::Patch],
            can_block: false,
            default_failure_policy: HookFailurePolicy::Ignore,
            allowed_failure_policies: vec![HookFailurePolicy::Ignore, HookFailurePolicy::FailTurn],
            default_recovery_mode: HookRecoveryMode::ReplayRequired,
            trace_requirements: HookTraceRequirements {
                include_name: true,
                include_hook_point: true,
                include_elapsed_ms: true,
                include_result_summary: true,
            },
            replay_requirements: HookReplayRequirements {
                include_hook_order: true,
                include_input_summary: true,
            },
            side_effect_persistence_requirements: HookSideEffectPersistenceRequirements {
                require_persistence_evidence: false,
                require_effect_summary: false,
            },
        }
    }

    #[test]
    fn capability_bridge_resolves_dotted_builtin_tool_calls_before_execution() {
        let runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_time".to_string()),
            name: "time.now".to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(
            execution
                .capability
                .as_ref()
                .map(|capability| capability.capability_id.as_str()),
            Some("builtin:time_now")
        );
        assert_eq!(execution.tool_call.name, "time.now");
        assert_eq!(execution.tool_result.tool_name, "time.now");
        assert_eq!(execution.tool_result.status, "ok");
        assert_eq!(execution.failure_kind, None);
    }

    #[test]
    fn runtime_hook_dispatch_returns_trace_records_in_priority_order() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.second",
                20,
                TurnHookPoint::ModelCallStart,
            ))
            .expect("register second hook");
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.first",
                10,
                TurnHookPoint::ModelCallStart,
            ))
            .expect("register first hook");

        let traces = runtime
            .dispatch_hook_trace_records(TurnHookPoint::ModelCallStart)
            .trace_records;

        assert_eq!(traces.len(), 2);
        assert_eq!(traces[0].hook_name, "observe.first");
        assert_eq!(traces[0].hook_order, 1);
        assert_eq!(traces[1].hook_name, "observe.second");
        assert_eq!(traces[1].hook_order, 2);
        assert!(traces
            .iter()
            .all(|trace| trace.hook_point == TurnHookPoint::ModelCallStart));
    }

    #[test]
    fn runtime_hook_dispatch_returns_empty_for_unregistered_boundary() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.model",
                10,
                TurnHookPoint::ModelCallStart,
            ))
            .expect("register model hook");

        let traces = runtime
            .dispatch_hook_trace_records(TurnHookPoint::ToolCallEnd)
            .trace_records;

        assert!(traces.is_empty());
    }

    #[test]
    fn runtime_hook_dispatch_records_executor_failure_without_stopping_turn_by_default() {
        let selection = test_provider_selection("http://localhost".to_string());
        let mut runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            Box::new(StubToolExecutor),
            Box::new(PassthroughPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.failure",
                10,
                TurnHookPoint::ModelCallStart,
            ))
            .expect("register failing hook");

        let outcome = runtime.dispatch_hook_trace_records(TurnHookPoint::ModelCallStart);
        let traces = outcome.trace_records;
        assert!(outcome.fail_turn_error.is_none());

        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].hook_name, "observe.failure");
        assert_eq!(traces[0].hook_order, 1);
        assert_eq!(traces[0].hook_point, TurnHookPoint::ModelCallStart);
        assert!(
            traces[0]
                .summary
                .contains("hook execution failed under ignore")
                || traces[0]
                    .summary
                    .contains("hook execution failed under Ignore")
        );
        assert!(traces[0]
            .input_summary
            .as_deref()
            .is_some_and(|summary| summary.contains("intentional hook failure")));
    }

    #[test]
    fn runtime_hook_dispatch_records_degrade_failure_evidence_without_stopping_turn() {
        let selection = test_provider_selection("http://localhost".to_string());
        let mut runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            Box::new(StubToolExecutor),
            Box::new(PassthroughPlanner),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        let mut descriptor =
            observe_hook_descriptor("observe.degrade-failure", 10, TurnHookPoint::ModelCallStart);
        descriptor.default_failure_policy = HookFailurePolicy::Degrade;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Ignore, HookFailurePolicy::Degrade];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register degrade hook");

        let outcome = runtime.dispatch_hook_trace_records(TurnHookPoint::ModelCallStart);
        let traces = outcome.trace_records;
        assert!(outcome.fail_turn_error.is_none());

        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].hook_name, "observe.degrade-failure");
        assert!(traces[0]
            .summary
            .contains("hook execution failed under Degrade"));
    }

    #[test]
    fn run_turn_records_planner_trace_records_in_terminal_trace() {
        let server = MockHttpServer::start(vec![json_completion("planner trace answer")]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let result = runtime.run_turn(TurnInput {
            message: "请总结当前状态".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("planner-trace-session".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        let _ = server.finish();

        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_point == TurnHookPoint::PlannerTurnPreflight));
        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_point == TurnHookPoint::PlannerToolSelection));

        let snapshot = runtime.load_session_snapshot(Some("planner-trace-session"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("planner trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.preflight.observe"));
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.tool_selection.observe"));
    }

    #[test]
    fn run_turn_records_capability_mediation_trace_for_forced_tool_planner() {
        let selection = test_provider_selection("http://localhost".to_string());
        let mut runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            Box::new(StubToolExecutor),
            Box::new(ForcedToolPlanner {
                tool_name: "workspace_list_files".to_string(),
                arguments: json!({}),
            }),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );

        let result = runtime.run_turn(TurnInput {
            message: "列出当前目录".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("capability-trace-session".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_point == TurnHookPoint::CapabilityResolve));

        let capability_trace = result
            .hook_trace_records
            .iter()
            .find(|record| record.hook_point == TurnHookPoint::CapabilityResolve)
            .expect("capability resolve trace should exist");
        assert!(capability_trace
            .summary
            .contains("capability mediation resolved"));

        let snapshot = runtime.load_session_snapshot(Some("capability-trace-session"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("capability trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "capability.resolve.observe"));
    }

    #[test]
    fn capability_mediation_hooks_can_rewrite_arguments_before_tool_execution() {
        let recorded_calls = Arc::new(Mutex::new(Vec::new()));
        let selection = test_provider_selection("http://localhost".to_string());
        let mut runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            Box::new(RecordingToolExecutor {
                calls: Arc::clone(&recorded_calls),
            }),
            Box::new(ForcedToolPlanner {
                tool_name: "workspace_list_files".to_string(),
                arguments: json!({"path":"."}),
            }),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        runtime.set_hook_executor_for_test(Box::new(TransformingCapabilityHookExecutor));
        runtime
            .register_hook_descriptor(transform_hook_descriptor(
                "capability.rewrite",
                10,
                TurnHookPoint::CapabilityResolve,
            ))
            .expect("register capability transform hook");

        let result = runtime.run_turn(TurnInput {
            message: "列出当前目录".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("capability-hook-rewrite".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "capability.rewrite"));
        let calls = recorded_calls.lock().unwrap();
        let call = calls.last().expect("tool call should be recorded");
        assert_eq!(
            call.arguments.get("path").and_then(Value::as_str),
            Some("src-tauri")
        );

        let snapshot = runtime.load_session_snapshot(Some("capability-hook-rewrite"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("capability hook trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "capability.rewrite"));
    }

    #[test]
    fn planner_preflight_hooks_can_rewrite_tool_call_before_execution() {
        let recorded_calls = Arc::new(Mutex::new(Vec::new()));
        let selection = test_provider_selection("http://localhost".to_string());
        let mut runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            Box::new(RecordingToolExecutor {
                calls: Arc::clone(&recorded_calls),
            }),
            Box::new(ForcedToolPlanner {
                tool_name: "workspace_list_files".to_string(),
                arguments: json!({"path":"."}),
            }),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        runtime.set_hook_executor_for_test(Box::new(TransformingPlannerHookExecutor));
        runtime
            .register_hook_descriptor(transform_hook_descriptor(
                "planner.preflight.rewrite",
                10,
                TurnHookPoint::PlannerTurnPreflight,
            ))
            .expect("register planner preflight transform hook");

        let result = runtime.run_turn(TurnInput {
            message: "列出当前目录".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("planner-preflight-rewrite".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.preflight.rewrite"));
        let calls = recorded_calls.lock().unwrap();
        let call = calls
            .last()
            .expect("planner preflight tool call should execute");
        assert_eq!(
            call.arguments.get("path").and_then(Value::as_str),
            Some("src-tauri")
        );

        let snapshot = runtime.load_session_snapshot(Some("planner-preflight-rewrite"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("planner preflight hook trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.preflight.rewrite"));
    }

    #[test]
    fn planner_tool_selection_hooks_can_rewrite_selected_tool_before_execution() {
        let recorded_calls = Arc::new(Mutex::new(Vec::new()));
        let selection = test_provider_selection("http://localhost".to_string());
        let mut runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            Box::new(RecordingToolExecutor {
                calls: Arc::clone(&recorded_calls),
            }),
            Box::new(ForcedToolPlanner {
                tool_name: "workspace_list_files".to_string(),
                arguments: json!({"path":"."}),
            }),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        runtime.set_hook_executor_for_test(Box::new(TransformingPlannerHookExecutor));
        runtime
            .register_hook_descriptor(transform_hook_descriptor(
                "planner.tool_selection.rewrite",
                10,
                TurnHookPoint::PlannerToolSelection,
            ))
            .expect("register planner tool-selection transform hook");

        let result = runtime.run_turn(TurnInput {
            message: "列出当前目录".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("planner-tool-selection-rewrite".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.tool_selection.rewrite"));
        let calls = recorded_calls.lock().unwrap();
        let call = calls
            .last()
            .expect("planner tool-selection call should execute");
        assert_eq!(
            call.arguments.get("path").and_then(Value::as_str),
            Some("tests")
        );

        let snapshot = runtime.load_session_snapshot(Some("planner-tool-selection-rewrite"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("planner tool-selection hook trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "planner.tool_selection.rewrite"));
    }

    #[test]
    fn skill_mediation_hooks_can_rewrite_arguments_before_skill_execution() {
        let recorded_calls = Arc::new(Mutex::new(Vec::new()));
        let selection = test_provider_selection("http://localhost".to_string());
        let mut runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver { selection }),
            Box::new(RecordingToolExecutor {
                calls: Arc::clone(&recorded_calls),
            }),
            Box::new(ForcedToolPlanner {
                tool_name: "echo_skill".to_string(),
                arguments: json!({"message":"original"}),
            }),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        runtime
            .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
                source: crate::agent::capability_bridge::SkillSourceView {
                    source_id: "builtin-skills".to_string(),
                    source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                    display_name: "Builtin Skills".to_string(),
                    availability:
                        crate::agent::capability_bridge::CapabilityAvailability::Available,
                    transport_kind: "host".to_string(),
                    server_identity: "skills://builtin".to_string(),
                    updated_at_ms: 1,
                    last_ingress_observation: None,
                },
                skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                    skill_id: "skill:echo".to_string(),
                    source_id: "builtin-skills".to_string(),
                    source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                    label: "echo_skill".to_string(),
                    description: "Echo message".to_string(),
                    input_schema_summary: "{}".to_string(),
                    safety_class: "".to_string(),
                    visibility: "default".to_string(),
                    observability_tags: vec![],
                    requires_approval: false,
                    host_mediated: false,
                    permission_scope: "".to_string(),
                    composed_capability_refs: vec!["builtin:echo_input".to_string()],
                    composed_capability_kinds: vec![
                        crate::agent::capability_bridge::CapabilityKind::Tool,
                    ],
                    executable_in_v1: true,
                }],
            })
            .expect("skill snapshot should apply");
        runtime.set_hook_executor_for_test(Box::new(TransformingCapabilityHookExecutor));
        runtime
            .register_hook_descriptor(transform_hook_descriptor(
                "skill.rewrite",
                10,
                TurnHookPoint::SkillToolActionsResolve,
            ))
            .expect("register skill transform hook");

        let result = runtime.run_turn(TurnInput {
            message: "运行 skill".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("skill-hook-rewrite".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "skill.rewrite"));
        let calls = recorded_calls.lock().unwrap();
        let call = calls.last().expect("skill tool call should be recorded");
        assert_eq!(
            call.arguments.get("message").and_then(Value::as_str),
            Some("patched by hook")
        );

        let snapshot = runtime.load_session_snapshot(Some("skill-hook-rewrite"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("skill hook trace should persist");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "skill.rewrite"));
    }

    #[test]
    fn start_turn_stream_does_not_dispatch_unstable_prepare_or_context_hooks() {
        let server = MockHttpServer::start(vec![sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "稳定边界答案。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 4,
                    "total_tokens": 16
                }
            }),
        ])]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.prepare-start",
                10,
                TurnHookPoint::TurnPrepareStart,
            ))
            .expect("register prepare start hook");
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.prepare-end",
                20,
                TurnHookPoint::TurnPrepareEnd,
            ))
            .expect("register prepare end hook");
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.context-start",
                30,
                TurnHookPoint::ContextBuildStart,
            ))
            .expect("register context start hook");
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.context-end",
                40,
                TurnHookPoint::ContextBuildEnd,
            ))
            .expect("register context end hook");
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.checkpoint-stable",
                50,
                TurnHookPoint::CheckpointPersistEnd,
            ))
            .expect("register checkpoint stable hook");
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-unstable-boundary-not-dispatched".to_string(),
            TurnInput {
                message: "请给出稳定边界测试答案。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("session-unstable-boundary-not-dispatched".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let _ = server.finish();

        let unstable_hook_names = [
            "observe.prepare-start",
            "observe.prepare-end",
            "observe.context-start",
            "observe.context-end",
        ];
        let event_hook_names = sink
            .events
            .borrow()
            .iter()
            .flat_map(|(_, payload)| payload.hook_trace_records.clone().unwrap_or_default())
            .map(|record| record.hook_name)
            .collect::<Vec<_>>();

        assert!(event_hook_names
            .iter()
            .any(|name| name == "observe.checkpoint-stable"));
        for unstable_hook_name in unstable_hook_names {
            assert!(
                !event_hook_names
                    .iter()
                    .any(|name| name == unstable_hook_name),
                "unstable hook `{unstable_hook_name}` should not be dispatched in streamed events"
            );
        }

        let snapshot =
            runtime.load_session_snapshot(Some("session-unstable-boundary-not-dispatched"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("persisted turn trace");
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.checkpoint-stable"));
        for unstable_hook_name in [
            "observe.prepare-start",
            "observe.prepare-end",
            "observe.context-start",
            "observe.context-end",
        ] {
            assert!(
                !trace
                    .hook_trace_records
                    .iter()
                    .any(|record| record.hook_name == unstable_hook_name),
                "unstable hook `{unstable_hook_name}` should not leak into persisted traces"
            );
        }
    }

    #[test]
    fn start_turn_stream_fail_turn_policy_emits_failed_terminal_with_hook_evidence() {
        let server = MockHttpServer::start(vec![]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        let mut descriptor =
            observe_hook_descriptor("observe.fail-turn", 10, TurnHookPoint::ModelCallStart);
        descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register fail-turn hook");
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-hook-failturn".to_string(),
            TurnInput {
                message: "请尝试开始一个会被 hook failturn 阻断的 turn".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("session-hook-failturn".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let request_bodies = server.finish();
        assert!(request_bodies.is_empty());

        let events = sink.events.borrow();
        let trace_event = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:trace").then_some(payload.clone()))
            .expect("trace event");
        let failed_event = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
            .expect("failed event");

        assert_eq!(trace_event.phase.as_deref(), Some("calling_model"));
        assert_eq!(failed_event.phase.as_deref(), Some("failed"));
        assert!(failed_event
            .error
            .as_deref()
            .is_some_and(|error| error.contains("observe.fail-turn")));
        assert_eq!(
            trace_event
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert_eq!(
            failed_event
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert!(failed_event
            .hook_trace_records
            .as_ref()
            .and_then(|records| records.first())
            .is_some_and(|record| record.blocked));
        assert_hook_boundary_alignment(
            &failed_event,
            TurnHookPoint::TurnFinalizeEnd,
            "turn.failed",
            "failed",
        );

        let snapshot = runtime.load_session_snapshot(Some("session-hook-failturn"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("persisted failed trace");
        assert_eq!(trace.phase, "failed");
        assert!(trace
            .error
            .as_deref()
            .is_some_and(|error| error.contains("observe.fail-turn")));
        assert_eq!(trace.hook_trace_records.len(), 1);
        assert!(trace.hook_trace_records[0].blocked);
    }

    #[test]
    fn start_turn_stream_fail_turn_policy_on_tool_call_start_stops_before_tool_execution() {
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
        ]);
        let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection(server.base_url.clone()),
            Box::new(CountingToolExecutor {
                calls: Arc::clone(&tool_calls),
            }),
        );
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        let mut descriptor = observe_hook_descriptor(
            "observe.tool-start-failturn",
            10,
            TurnHookPoint::ToolCallStart,
        );
        descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register tool-start failturn hook");
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-tool-start-failturn".to_string(),
            TurnInput {
                message: "请先列出文件。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("session-tool-start-failturn".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let request_bodies = server.finish();
        assert!((1..=2).contains(&request_bodies.len()));
        assert_eq!(tool_calls.load(AtomicOrdering::SeqCst), 0);

        let events = sink.events.borrow();
        let tool_started = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:tool"
                    && payload.event_type.as_deref() == Some("turn.tool_call_started"))
                .then_some(payload.clone())
            })
            .expect("tool started event");
        let failed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
            .expect("failed event");

        assert_eq!(
            tool_started
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert_eq!(
            failed
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert!(failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("observe.tool-start-failturn")));

        let snapshot = runtime.load_session_snapshot(Some("session-tool-start-failturn"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("persisted failed trace");
        assert_eq!(trace.phase, "failed");
        assert_eq!(trace.hook_trace_records.len(), 1);
        assert!(trace.hook_trace_records[0].blocked);
    }

    #[test]
    fn start_turn_stream_fail_turn_policy_on_tool_call_end_stops_before_followup_model_call() {
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
        ]);
        let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection(server.base_url.clone()),
            Box::new(CountingToolExecutor {
                calls: Arc::clone(&tool_calls),
            }),
        );
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        let mut descriptor =
            observe_hook_descriptor("observe.tool-end-failturn", 10, TurnHookPoint::ToolCallEnd);
        descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register tool-end failturn hook");
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-tool-end-failturn".to_string(),
            TurnInput {
                message: "请先列出文件。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("session-tool-end-failturn".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let request_bodies = server.finish();
        assert!((1..=2).contains(&request_bodies.len()));
        assert_eq!(tool_calls.load(AtomicOrdering::SeqCst), 1);

        let events = sink.events.borrow();
        let tool_completed = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:tool"
                    && payload.event_type.as_deref() == Some("turn.tool_call_completed"))
                .then_some(payload.clone())
            })
            .expect("tool completed event");
        let failed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
            .expect("failed event");

        assert_eq!(
            tool_completed
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert_eq!(
            failed
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert!(failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("observe.tool-end-failturn")));

        let snapshot = runtime.load_session_snapshot(Some("session-tool-end-failturn"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("persisted failed trace");
        assert_eq!(trace.phase, "failed");
        assert_eq!(trace.hook_trace_records.len(), 1);
        assert!(trace.hook_trace_records[0].blocked);
    }

    #[test]
    fn start_turn_stream_fail_turn_policy_on_checkpoint_boundary_emits_failed_instead_of_completed()
    {
        let server = MockHttpServer::start(vec![sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "checkpoint failturn answer"
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 4,
                    "total_tokens": 16
                }
            }),
        ])]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        let mut descriptor = observe_hook_descriptor(
            "observe.checkpoint-failturn",
            10,
            TurnHookPoint::CheckpointPersistEnd,
        );
        descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register checkpoint failturn hook");
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-checkpoint-failturn".to_string(),
            TurnInput {
                message: "请回答一个会在 checkpoint boundary failturn 的问题".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("session-checkpoint-failturn".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let _ = server.finish();
        let events = sink.events.borrow();
        assert!(!events.iter().any(|(name, _)| name == "turn:completed"));
        let checkpoint = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:checkpoint_persisted").then_some(payload.clone())
            })
            .expect("checkpoint persisted event");
        let failed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
            .expect("failed event");

        assert_eq!(
            checkpoint
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert_eq!(
            failed
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert!(failed
            .error
            .as_deref()
            .is_some_and(|error| error.contains("observe.checkpoint-failturn")));

        let snapshot = runtime.load_session_snapshot(Some("session-checkpoint-failturn"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("persisted failed trace");
        assert_eq!(trace.phase, "failed");
        assert_eq!(trace.hook_trace_records.len(), 1);
        assert!(trace.hook_trace_records[0].blocked);
    }

    #[test]
    fn start_turn_stream_fail_turn_policy_on_finalize_boundary_emits_failed_with_terminal_hook_evidence(
    ) {
        let server = MockHttpServer::start(vec![sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "finalize failturn answer"
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 12,
                    "completion_tokens": 4,
                    "total_tokens": 16
                }
            }),
        ])]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.checkpoint-ok",
                10,
                TurnHookPoint::CheckpointPersistEnd,
            ))
            .expect("register checkpoint observe hook");
        let mut descriptor = observe_hook_descriptor(
            "observe.finalize-failturn",
            20,
            TurnHookPoint::TurnFinalizeEnd,
        );
        descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register finalize failturn hook");
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-finalize-failturn".to_string(),
            TurnInput {
                message: "请回答一个会在 finalize boundary failturn 的问题".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("session-finalize-failturn".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let _ = server.finish();
        let events = sink.events.borrow();
        assert!(!events.iter().any(|(name, _)| name == "turn:completed"));
        let checkpoint = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:checkpoint_persisted").then_some(payload.clone())
            })
            .expect("checkpoint persisted event");
        let failed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
            .expect("failed event");

        assert_eq!(
            checkpoint
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert_eq!(
            failed
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(2)
        );
        assert!(failed
            .hook_trace_records
            .as_ref()
            .is_some_and(|records| records.iter().any(|record| {
                record.hook_name == "observe.finalize-failturn" && record.blocked
            })));

        let snapshot = runtime.load_session_snapshot(Some("session-finalize-failturn"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("persisted failed trace");
        assert_eq!(trace.phase, "failed");
        assert_eq!(trace.hook_trace_records.len(), 2);
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.finalize-failturn" && record.blocked));
    }

    #[test]
    fn run_turn_fail_turn_policy_on_model_call_start_returns_failed_result_with_hook_evidence() {
        let server = MockHttpServer::start(vec![]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        let mut descriptor = observe_hook_descriptor(
            "observe.sync-model-failturn",
            10,
            TurnHookPoint::ModelCallStart,
        );
        descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register sync model failturn hook");

        let result = runtime.run_turn(TurnInput {
            message: "请尝试一个同步 failturn turn".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-model-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        let request_bodies = server.finish();
        assert!(request_bodies.is_empty());
        assert_eq!(result.phase, "failed");
        assert_eq!(result.hook_trace_records.len(), 1);
        assert!(result.hook_trace_records[0].blocked);
        assert!(result
            .assistant_message
            .contains("observe.sync-model-failturn"));
    }

    #[test]
    fn run_turn_fail_turn_policy_on_tool_call_start_returns_failed_before_tool_execution() {
        let server = MockHttpServer::start(vec![json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        ))]);
        let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection(server.base_url.clone()),
            Box::new(CountingToolExecutor {
                calls: Arc::clone(&tool_calls),
            }),
        );
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        let mut descriptor = observe_hook_descriptor(
            "observe.sync-tool-start-failturn",
            10,
            TurnHookPoint::ToolCallStart,
        );
        descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register sync tool-start failturn hook");

        let result = runtime.run_turn(TurnInput {
            message: "请先列出文件。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-tool-start-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        let request_bodies = server.finish();
        assert_eq!(request_bodies.len(), 1);
        assert_eq!(tool_calls.load(AtomicOrdering::SeqCst), 0);
        assert_eq!(result.phase, "failed");
        assert!(result
            .assistant_message
            .contains("observe.sync-tool-start-failturn"));
        assert!(result
            .hook_trace_records
            .iter()
            .any(
                |record| record.hook_name == "observe.sync-tool-start-failturn" && record.blocked
            ));
    }

    #[test]
    fn run_turn_fail_turn_policy_on_tool_call_end_returns_failed_before_followup_model_call() {
        let server = MockHttpServer::start(vec![json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        ))]);
        let tool_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection(server.base_url.clone()),
            Box::new(CountingToolExecutor {
                calls: Arc::clone(&tool_calls),
            }),
        );
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        let mut descriptor = observe_hook_descriptor(
            "observe.sync-tool-end-failturn",
            10,
            TurnHookPoint::ToolCallEnd,
        );
        descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register sync tool-end failturn hook");

        let result = runtime.run_turn(TurnInput {
            message: "请先列出文件。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-tool-end-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        let request_bodies = server.finish();
        assert_eq!(request_bodies.len(), 1);
        assert_eq!(tool_calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(result.phase, "failed");
        assert!(result
            .assistant_message
            .contains("observe.sync-tool-end-failturn"));
        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.sync-tool-end-failturn" && record.blocked));
    }

    #[test]
    fn run_turn_persists_terminal_hook_traces_on_completed_sync_turn() {
        let server = MockHttpServer::start(vec![json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "同步完成答案。"
                    }
                }
            ],
            "usage": {
                "prompt_tokens": 16,
                "completion_tokens": 5,
                "total_tokens": 21
            }
        }))]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.sync-checkpoint",
                10,
                TurnHookPoint::CheckpointPersistEnd,
            ))
            .expect("register sync checkpoint hook");
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.sync-finalize",
                20,
                TurnHookPoint::TurnFinalizeEnd,
            ))
            .expect("register sync finalize hook");

        let result = runtime.run_turn(TurnInput {
            message: "请直接同步回答。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-hook-trace-terminal".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        let request_bodies = server.finish();
        assert_eq!(request_bodies.len(), 1);
        assert_eq!(result.phase, "ready");
        assert!(result
            .event_id
            .as_deref()
            .is_some_and(|value| value.starts_with("sync:sync-hook-trace-terminal:")));
        assert_eq!(result.event_type.as_deref(), Some("turn.completed"));
        assert_eq!(result.event_version.as_deref(), Some("turn-event-v1"));
        assert_eq!(result.sequence, Some(1));
        assert!(result.emitted_at_ms.is_some());
        assert_eq!(result.hook_trace_records.len(), 2);
        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.sync-checkpoint"
                && record.hook_point == TurnHookPoint::CheckpointPersistEnd));
        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.sync-finalize"
                && record.hook_point == TurnHookPoint::TurnFinalizeEnd));

        let snapshot = runtime.load_session_snapshot(Some("sync-hook-trace-terminal"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("persisted sync trace");
        assert_eq!(trace.phase, "completed");
        assert!(trace
            .event_id
            .as_deref()
            .is_some_and(|value| value.starts_with("sync:sync-hook-trace-terminal:")));
        assert_eq!(trace.event_type.as_deref(), Some("turn.completed"));
        assert_eq!(trace.event_version.as_deref(), Some("turn-event-v1"));
        assert_eq!(trace.sequence, Some(1));
        assert!(trace.emitted_at_ms.is_some());
        assert_eq!(trace.hook_trace_records.len(), 2);
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.sync-checkpoint"
                && record.hook_point == TurnHookPoint::CheckpointPersistEnd));
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.sync-finalize"
                && record.hook_point == TurnHookPoint::TurnFinalizeEnd));
    }

    #[test]
    fn run_turn_fail_turn_policy_on_checkpoint_boundary_persists_failed_sync_trace() {
        let server = MockHttpServer::start(vec![json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "checkpoint failturn answer"
                    }
                }
            ]
        }))]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        let mut descriptor = observe_hook_descriptor(
            "observe.sync-checkpoint-failturn",
            10,
            TurnHookPoint::CheckpointPersistEnd,
        );
        descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register sync checkpoint failturn hook");

        let result = runtime.run_turn(TurnInput {
            message: "请回答一个会在 sync checkpoint failturn 的问题".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-checkpoint-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        let request_bodies = server.finish();
        assert_eq!(request_bodies.len(), 1);
        assert_eq!(result.phase, "failed");
        assert!(result
            .event_id
            .as_deref()
            .is_some_and(|value| value.starts_with("sync:sync-checkpoint-failturn:")));
        assert_eq!(result.event_type.as_deref(), Some("turn.failed"));
        assert_eq!(result.event_version.as_deref(), Some("turn-event-v1"));
        assert_eq!(result.sequence, Some(1));
        assert!(result.emitted_at_ms.is_some());
        assert_eq!(result.hook_trace_records.len(), 1);
        assert!(result.hook_trace_records[0].blocked);
        assert!(result
            .assistant_message
            .contains("observe.sync-checkpoint-failturn"));

        let snapshot = runtime.load_session_snapshot(Some("sync-checkpoint-failturn"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("persisted failed sync trace");
        assert_eq!(trace.phase, "failed");
        assert!(trace
            .event_id
            .as_deref()
            .is_some_and(|value| value.starts_with("sync:sync-checkpoint-failturn:")));
        assert_eq!(trace.event_type.as_deref(), Some("turn.failed"));
        assert_eq!(trace.event_version.as_deref(), Some("turn-event-v1"));
        assert_eq!(trace.sequence, Some(1));
        assert!(trace.emitted_at_ms.is_some());
        assert_eq!(trace.hook_trace_records.len(), 1);
        assert!(trace.hook_trace_records[0].blocked);
        assert!(trace
            .error
            .as_deref()
            .is_some_and(|error| error.contains("observe.sync-checkpoint-failturn")));
    }

    #[test]
    fn run_turn_fail_turn_policy_on_finalize_boundary_persists_terminal_sync_hook_evidence() {
        let server = MockHttpServer::start(vec![json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "finalize failturn answer"
                    }
                }
            ]
        }))]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        runtime.set_hook_executor_for_test(Box::new(FailingHookExecutor));
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.sync-checkpoint-ok",
                10,
                TurnHookPoint::CheckpointPersistEnd,
            ))
            .expect("register sync checkpoint observe hook");
        let mut descriptor = observe_hook_descriptor(
            "observe.sync-finalize-failturn",
            20,
            TurnHookPoint::TurnFinalizeEnd,
        );
        descriptor.default_failure_policy = HookFailurePolicy::FailTurn;
        descriptor.allowed_failure_policies =
            vec![HookFailurePolicy::Degrade, HookFailurePolicy::FailTurn];
        runtime
            .register_hook_descriptor(descriptor)
            .expect("register sync finalize failturn hook");

        let result = runtime.run_turn(TurnInput {
            message: "请回答一个会在 sync finalize failturn 的问题".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-finalize-failturn".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        let request_bodies = server.finish();
        assert_eq!(request_bodies.len(), 1);
        assert_eq!(result.phase, "failed");
        assert!(result
            .event_id
            .as_deref()
            .is_some_and(|value| value.starts_with("sync:sync-finalize-failturn:")));
        assert_eq!(result.event_type.as_deref(), Some("turn.failed"));
        assert_eq!(result.event_version.as_deref(), Some("turn-event-v1"));
        assert_eq!(result.sequence, Some(1));
        assert!(result.emitted_at_ms.is_some());
        assert_eq!(result.hook_trace_records.len(), 2);
        assert!(result
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.sync-checkpoint-ok"
                && record.hook_point == TurnHookPoint::CheckpointPersistEnd));
        assert!(result
            .hook_trace_records
            .iter()
            .any(
                |record| record.hook_name == "observe.sync-finalize-failturn"
                    && record.hook_point == TurnHookPoint::TurnFinalizeEnd
                    && record.blocked
            ));

        let snapshot = runtime.load_session_snapshot(Some("sync-finalize-failturn"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("persisted failed sync trace");
        assert_eq!(trace.phase, "failed");
        assert!(trace
            .event_id
            .as_deref()
            .is_some_and(|value| value.starts_with("sync:sync-finalize-failturn:")));
        assert_eq!(trace.event_type.as_deref(), Some("turn.failed"));
        assert_eq!(trace.event_version.as_deref(), Some("turn-event-v1"));
        assert_eq!(trace.sequence, Some(1));
        assert!(trace.emitted_at_ms.is_some());
        assert_eq!(trace.hook_trace_records.len(), 2);
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.sync-finalize-failturn" && record.blocked));
    }

    #[test]
    fn start_turn_stream_persists_terminal_hook_traces_on_stable_boundaries() {
        let server = MockHttpServer::start(vec![sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "先想一下。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "最终答案。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 6,
                    "total_tokens": 26
                }
            }),
        ])]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.checkpoint",
                10,
                TurnHookPoint::CheckpointPersistEnd,
            ))
            .expect("register checkpoint hook");
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.finalize",
                20,
                TurnHookPoint::TurnFinalizeEnd,
            ))
            .expect("register finalize hook");
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-hook-trace-terminal".to_string(),
            TurnInput {
                message: "请直接回答。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-hook-trace-terminal".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let _ = server.finish();
        {
            let events = sink.events.borrow();
            let checkpoint_persisted = events
                .iter()
                .find_map(|(name, payload)| {
                    (name == "turn:checkpoint_persisted"
                        && payload.event_type.as_deref() == Some("turn.checkpoint_persisted"))
                    .then_some(payload.clone())
                })
                .expect("checkpoint persisted event");
            let completed = events
                .iter()
                .find_map(|(name, payload)| {
                    (name == "turn:completed"
                        && payload.event_type.as_deref() == Some("turn.completed"))
                    .then_some(payload.clone())
                })
                .expect("completed event");

            let checkpoint_records = checkpoint_persisted
                .hook_trace_records
                .clone()
                .expect("checkpoint hook traces");
            assert_eq!(checkpoint_records.len(), 1);
            assert_eq!(checkpoint_records[0].hook_name, "observe.checkpoint");
            assert_eq!(
                checkpoint_records[0].hook_point,
                TurnHookPoint::CheckpointPersistEnd
            );

            let completed_records = completed
                .hook_trace_records
                .clone()
                .expect("completed hook traces");
            assert_eq!(completed_records.len(), 2);
            assert!(completed_records
                .iter()
                .any(|record| record.hook_name == "observe.checkpoint"
                    && record.hook_point == TurnHookPoint::CheckpointPersistEnd));
            assert!(completed_records
                .iter()
                .any(|record| record.hook_name == "observe.finalize"
                    && record.hook_point == TurnHookPoint::TurnFinalizeEnd));
        }

        let snapshot = runtime.load_session_snapshot(Some("stream-hook-trace-terminal"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("persisted turn trace");
        assert_eq!(trace.hook_trace_records.len(), 2);
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.checkpoint"
                && record.hook_point == TurnHookPoint::CheckpointPersistEnd));
        assert!(trace
            .hook_trace_records
            .iter()
            .any(|record| record.hook_name == "observe.finalize"
                && record.hook_point == TurnHookPoint::TurnFinalizeEnd));
    }

    #[test]
    fn capability_bridge_returns_normalized_not_found_failure_for_unknown_tools() {
        let runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_unknown".to_string()),
            name: "unknown_tool".to_string(),
            arguments: json!({ "path": "src" }),
            plan: None,
        });

        assert!(execution.capability.is_none());
        assert_eq!(execution.tool_result.status, "error");
        assert_eq!(
            execution.failure_kind,
            Some(CapabilityFailureKind::CapabilityNotFound)
        );
        assert!(execution.tool_result.output.contains("capability registry"));
    }

    #[test]
    fn capability_bridge_resolves_host_registered_mcp_tool_snapshot() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Local MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://local".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:tool:workspace_search".to_string(),
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "workspace_search".to_string(),
                description: "List files through MCP".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                input_schema_summary: "{\"path\":\"string\"}".to_string(),
                safety_class: "host_tool".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string(), "tool".to_string()],
                requires_approval: false,
                host_mediated: true,
                permission_scope: "workspace.read".to_string(),
            }],
        });

        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_workspace_search".to_string()),
            name: "workspace_search".to_string(),
            arguments: json!({ "path": "." }),
            plan: None,
        });

        assert_eq!(
            execution
                .capability
                .as_ref()
                .map(|capability| capability.capability_id.as_str()),
            Some("mcp:tool:workspace_search")
        );
        assert_eq!(execution.tool_result.status, "ok");
        assert_eq!(execution.failure_kind, None);
    }

    #[test]
    fn capability_bridge_keeps_mcp_as_runtime_ingress_not_planner_scheduler_state() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Local MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://local".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:tool:workspace_search".to_string(),
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "workspace_search".to_string(),
                description: "List files through MCP".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                input_schema_summary: "{\"query\":\"string\"}".to_string(),
                safety_class: "host_tool".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string(), "tool".to_string()],
                requires_approval: false,
                host_mediated: true,
                permission_scope: "workspace.read".to_string(),
            }],
        });

        let planner = LocalTurnPlanner;
        let provider_tool_call = ToolCall {
            call_id: Some("call_workspace_search".to_string()),
            name: "workspace_search".to_string(),
            arguments: json!({ "query": "Cargo.toml" }),
            plan: None,
        };
        let planned = planner
            .select_tool_call(
                "搜索 Cargo.toml",
                &Vec::<TurnHistoryMessage>::new(),
                &[],
                Some(provider_tool_call),
            )
            .expect("planner should preserve provider tool call");

        assert_eq!(planned.name, "workspace_search");
        assert_eq!(planned.arguments, json!({ "query": "Cargo.toml" }));
        assert!(planned.arguments.get("sourceId").is_none());
        assert!(planned.arguments.get("transport").is_none());
        assert!(planned.arguments.get("capabilityId").is_none());

        let execution = runtime.execute_capability_tool_call(&planned);

        assert_eq!(
            execution
                .capability
                .as_ref()
                .map(|capability| capability.capability_id.as_str()),
            Some("mcp:tool:workspace_search")
        );
        assert_eq!(execution.tool_call.name, "workspace_search");
        assert_eq!(
            execution.tool_call.arguments,
            json!({ "query": "Cargo.toml" })
        );
        assert_eq!(execution.tool_result.status, "ok");
    }

    #[test]
    fn capability_bridge_propagates_source_unavailable_from_runtime_execution_path() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-offline".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Offline MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://offline".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Unreachable,
                declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:tool:offline_search".to_string(),
                source_id: "mcp-offline".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "offline_search".to_string(),
                description: "Offline search".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                input_schema_summary: "{}".to_string(),
                safety_class: "host_tool".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string(), "tool".to_string()],
                requires_approval: false,
                host_mediated: true,
                permission_scope: "workspace.read".to_string(),
            }],
        });

        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_offline_search".to_string()),
            name: "offline_search".to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(
            execution.failure_kind,
            Some(CapabilityFailureKind::SourceUnavailable)
        );
        assert!(execution.tool_result.output.contains("source"));
    }

    #[test]
    fn capability_bridge_propagates_permission_denied_from_runtime_execution_path() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-approval".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Approval MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://approval".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
                permission_profile: "requires-approval".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:tool:guarded_search".to_string(),
                source_id: "mcp-approval".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "guarded_search".to_string(),
                description: "Guarded search".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                input_schema_summary: "{}".to_string(),
                safety_class: "host_tool".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string(), "tool".to_string()],
                requires_approval: true,
                host_mediated: false,
                permission_scope: "workspace.read".to_string(),
            }],
        });

        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_guarded_search".to_string()),
            name: "guarded_search".to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(
            execution.failure_kind,
            Some(CapabilityFailureKind::PermissionDenied)
        );
        assert!(execution.tool_result.output.contains("审批"));
    }

    #[test]
    fn capability_bridge_propagates_malformed_response_from_runtime_execution_path() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.register_mcp_capability_for_test(crate::agent::capability_bridge::CapabilityView {
            capability_id: "mcp:tool:orphaned".to_string(),
            source_id: "mcp-missing".to_string(),
            source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
            kind: crate::agent::capability_bridge::CapabilityKind::Tool,
            label: "orphaned_tool".to_string(),
            description: "Orphaned tool".to_string(),
            invocation_mode:
                crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
            input_schema_summary: "{}".to_string(),
            safety_class: "host_tool".to_string(),
            visibility: "default".to_string(),
            observability_tags: vec!["mcp".to_string(), "tool".to_string()],
            requires_approval: false,
            host_mediated: true,
            permission_scope: "workspace.read".to_string(),
        });
        runtime.remove_mcp_source_for_test("mcp-missing");

        let execution = runtime.execute_capability_tool_call(&ToolCall {
            call_id: Some("call_orphaned_tool".to_string()),
            name: "orphaned_tool".to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(
            execution.failure_kind,
            Some(CapabilityFailureKind::MalformedResponse)
        );
        assert!(execution.tool_result.output.contains("registry"));
    }

    #[test]
    fn skill_bridge_executes_tool_only_skill_without_second_scheduler() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-skills".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Skills MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://skills".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:tool:workspace_search".to_string(),
                source_id: "mcp-skills".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "workspace_search".to_string(),
                description: "Search workspace".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                input_schema_summary: "{}".to_string(),
                safety_class: "host_tool".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string(), "tool".to_string()],
                requires_approval: false,
                host_mediated: true,
                permission_scope: "workspace.read".to_string(),
            }],
        });
        runtime
            .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
                source: crate::agent::capability_bridge::SkillSourceView {
                    source_id: "host-skills".to_string(),
                    source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                    display_name: "Host Skills".to_string(),
                    availability:
                        crate::agent::capability_bridge::CapabilityAvailability::Available,
                    transport_kind: "host".to_string(),
                    server_identity: "skills://host".to_string(),
                    updated_at_ms: 2,
                    last_ingress_observation: None,
                },
                skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                    skill_id: "skill:search".to_string(),
                    source_id: "host-skills".to_string(),
                    source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                    label: "search".to_string(),
                    description: "Search workspace".to_string(),
                    input_schema_summary: "{}".to_string(),
                    safety_class: "".to_string(),
                    visibility: "default".to_string(),
                    observability_tags: vec!["host".to_string()],
                    requires_approval: false,
                    host_mediated: false,
                    permission_scope: "".to_string(),
                    composed_capability_refs: vec!["mcp:tool:workspace_search".to_string()],
                    composed_capability_kinds: vec![],
                    executable_in_v1: false,
                }],
            })
            .expect("skill snapshot should apply");

        let execution = runtime.execute_skill_tool_call(&SkillInvocationRequest {
            skill_id: "skill:search".to_string(),
            arguments: json!({ "query": "Cargo.toml" }),
        });

        assert_eq!(execution.failure_layer, None);
        assert_eq!(execution.capability_executions.len(), 1);
        assert_eq!(
            execution
                .skill
                .as_ref()
                .map(|skill| skill.composed_capability_refs.as_slice()),
            Some(["mcp:tool:workspace_search".to_string()].as_slice())
        );
        assert_eq!(
            execution.capability_executions[0].tool_call.arguments,
            json!({ "query": "Cargo.toml" })
        );
        assert!(execution.capability_executions[0]
            .invocation_record_with_skill_context(
                execution.skill.as_ref(),
                execution.failure_layer.as_ref()
            )
            .skill_id
            .is_some());
    }

    #[test]
    fn runtime_executes_registered_skill_by_tool_name() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime
            .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
                source: crate::agent::capability_bridge::SkillSourceView {
                    source_id: "builtin-skills".to_string(),
                    source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                    display_name: "Builtin Skills".to_string(),
                    availability:
                        crate::agent::capability_bridge::CapabilityAvailability::Available,
                    transport_kind: "host".to_string(),
                    server_identity: "skills://builtin".to_string(),
                    updated_at_ms: 1,
                    last_ingress_observation: None,
                },
                skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                    skill_id: "skill:clock".to_string(),
                    source_id: "builtin-skills".to_string(),
                    source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                    label: "clock".to_string(),
                    description: "Get current time".to_string(),
                    input_schema_summary: "{}".to_string(),
                    safety_class: "".to_string(),
                    visibility: "default".to_string(),
                    observability_tags: vec![],
                    requires_approval: false,
                    host_mediated: false,
                    permission_scope: "".to_string(),
                    composed_capability_refs: vec!["builtin:time_now".to_string()],
                    composed_capability_kinds: vec![],
                    executable_in_v1: false,
                }],
            })
            .expect("skill snapshot should apply");

        let (tool_result, invocation_record, hook_trace_records) = runtime
            .execute_registered_tool_call(&ToolCall {
                call_id: None,
                name: "clock".to_string(),
                arguments: json!({}),
                plan: None,
            });

        assert_eq!(tool_result.status, "ok");
        assert_eq!(invocation_record.skill_id.as_deref(), Some("skill:clock"));
        assert_eq!(invocation_record.failure_layer.as_deref(), None);
        assert_eq!(hook_trace_records.len(), 1);
        assert_eq!(
            hook_trace_records[0].hook_point,
            TurnHookPoint::SkillToolActionsResolve
        );
    }

    #[test]
    fn skill_bridge_rejects_non_tool_composed_skill_as_unsupported() {
        let mut runtime =
            build_runtime_for_test(test_provider_selection("http://localhost".to_string()));
        runtime.apply_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-skills".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Skills MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://skills".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                declared_capabilities: vec![
                    crate::agent::capability_bridge::CapabilityKind::Resource,
                ],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: 1,
                last_ingress_observation: None,
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:resource:repo_index".to_string(),
                source_id: "mcp-skills".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Resource,
                label: "repo_index".to_string(),
                description: "Repository index".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::ReadOnlyFetch,
                input_schema_summary: "{}".to_string(),
                safety_class: "read_only".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string(), "resource".to_string()],
                requires_approval: false,
                host_mediated: true,
                permission_scope: "workspace.read".to_string(),
            }],
        });
        runtime
            .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
                source: crate::agent::capability_bridge::SkillSourceView {
                    source_id: "host-skills".to_string(),
                    source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                    display_name: "Host Skills".to_string(),
                    availability:
                        crate::agent::capability_bridge::CapabilityAvailability::Available,
                    transport_kind: "host".to_string(),
                    server_identity: "skills://host".to_string(),
                    updated_at_ms: 2,
                    last_ingress_observation: None,
                },
                skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                    skill_id: "skill:index".to_string(),
                    source_id: "host-skills".to_string(),
                    source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                    label: "index".to_string(),
                    description: "Index repository".to_string(),
                    input_schema_summary: "{}".to_string(),
                    safety_class: "".to_string(),
                    visibility: "default".to_string(),
                    observability_tags: vec![],
                    requires_approval: false,
                    host_mediated: false,
                    permission_scope: "".to_string(),
                    composed_capability_refs: vec!["mcp:resource:repo_index".to_string()],
                    composed_capability_kinds: vec![],
                    executable_in_v1: false,
                }],
            })
            .expect("skill snapshot should apply");

        let execution = runtime.execute_skill_tool_call(&SkillInvocationRequest {
            skill_id: "skill:index".to_string(),
            arguments: json!({ "path": "." }),
        });

        assert_eq!(
            execution.failure_layer,
            Some(SkillFailureLayer::UnsupportedComposition)
        );
        assert_eq!(
            execution.capability_executions[0].tool_result.status,
            "error"
        );
    }

    #[test]
    fn skill_bridge_propagates_underlying_capability_execution_failure() {
        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection("http://localhost".to_string()),
            Box::new(ErrorToolExecutor),
        );
        runtime
            .apply_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
                source: crate::agent::capability_bridge::SkillSourceView {
                    source_id: "builtin-skills".to_string(),
                    source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                    display_name: "Builtin Skills".to_string(),
                    availability:
                        crate::agent::capability_bridge::CapabilityAvailability::Available,
                    transport_kind: "host".to_string(),
                    server_identity: "skills://builtin".to_string(),
                    updated_at_ms: 1,
                    last_ingress_observation: None,
                },
                skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                    skill_id: "skill:read-file".to_string(),
                    source_id: "builtin-skills".to_string(),
                    source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                    label: "read-file".to_string(),
                    description: "Read a file".to_string(),
                    input_schema_summary: "{\"path\":\"string\"}".to_string(),
                    safety_class: "".to_string(),
                    visibility: "default".to_string(),
                    observability_tags: vec![],
                    requires_approval: false,
                    host_mediated: false,
                    permission_scope: "".to_string(),
                    composed_capability_refs: vec!["builtin:workspace_read_file".to_string()],
                    composed_capability_kinds: vec![],
                    executable_in_v1: false,
                }],
            })
            .expect("skill snapshot should apply");

        let execution = runtime.execute_skill_tool_call(&SkillInvocationRequest {
            skill_id: "skill:read-file".to_string(),
            arguments: json!({ "path": "missing-file-for-skill-bridge-test.txt" }),
        });

        assert_eq!(
            execution.failure_layer,
            Some(SkillFailureLayer::UnderlyingCapabilityExecution)
        );
        assert_eq!(
            execution.capability_executions[0].failure_kind,
            Some(CapabilityFailureKind::InvocationFailed)
        );
    }

    fn temp_marker_file_path(prefix: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{stamp}.tmp"))
    }

    fn decision_tool_call(tool_name: &str, arguments: serde_json::Value) -> serde_json::Value {
        json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": format!("先调用 {}。", tool_name),
                        "reasoning_content": format!("需要先执行 {}。", tool_name),
                        "tool_calls": [
                            {
                                "id": format!("call_{}", tool_name),
                                "type": "function",
                                "function": {
                                    "name": tool_name,
                                    "arguments": arguments.to_string()
                                }
                            }
                        ]
                    }
                }
            ]
        })
    }

    fn decision_blank_tool_call(arguments: serde_json::Value) -> serde_json::Value {
        json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "继续读取目标文件。",
                        "reasoning_content": "参数已经足够，继续执行下一步读取。",
                        "tool_calls": [
                            {
                                "id": "call_blank_name",
                                "type": "function",
                                "function": {
                                    "name": "",
                                    "arguments": arguments.to_string()
                                }
                            }
                        ]
                    }
                }
            ]
        })
    }

    #[test]
    fn start_turn_stream_uses_sink_for_empty_input_failure() {
        let mut runtime = AgentRuntime::new();
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-empty".to_string(),
            TurnInput {
                message: "   ".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("test-session".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let events = sink.events.borrow();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, "turn:failed");
        assert_eq!(events[0].1.turn_id, "turn-empty");
        assert_eq!(events[0].1.phase.as_deref(), Some("failed"));
        assert_eq!(events[0].1.error.as_deref(), Some("Message is empty."));
        assert_hook_boundary_alignment(
            &events[0].1,
            TurnHookPoint::TurnFinalizeEnd,
            "turn.failed",
            "failed",
        );
    }

    #[test]
    fn start_turn_stream_can_emit_cancelled_when_stop_requested_before_plan() {
        let selection = test_provider_selection("http://127.0.0.1:1/v1".to_string());
        let mut runtime = build_runtime_for_test(selection);
        let sink = RecordingTurnEventSink::new();
        let control = ExecutionControlRegistry::new();

        control.register_turn("turn-cancelled", Some("stop-session"), None);
        let response = control.request_stop("turn-cancelled");
        assert!(response.accepted);

        runtime.start_turn_stream_with_control(
            &sink,
            &control,
            "turn-cancelled".to_string(),
            TurnInput {
                message: "继续读取 tauri.conf.json".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stop-session".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let events = sink.events.borrow();
        assert!(events.iter().any(|(name, payload)| {
            name == "turn:cancelled"
                && payload.turn_id == "turn-cancelled"
                && payload.error.as_deref() == Some("stopped_by_user")
        }));
        let cancelled = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:cancelled").then_some(payload.clone()))
            .expect("cancelled event");
        assert_hook_boundary_alignment(
            &cancelled,
            TurnHookPoint::TurnFinalizeEnd,
            "turn.cancelled",
            "cancelled",
        );

        let snapshot = runtime.load_session_snapshot(Some("stop-session"));
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].role, "user");
        assert_eq!(snapshot.history[0].content, "继续读取 tauri.conf.json");
        assert_eq!(snapshot.history[1].role, "assistant");
        assert_eq!(snapshot.history[1].content, CANCELLED_TURN_MESSAGE);
    }

    #[test]
    fn runtime_can_build_graph_turn_handoff_from_stable_turn_artifacts() {
        let selection = test_provider_selection("http://127.0.0.1:1/v1".to_string());
        let mut runtime = build_runtime_for_test(selection);
        runtime.load_session_snapshot(Some("graph-session"));
        let result = TurnResult {
            event_id: None,
            event_type: None,
            event_version: None,
            sequence: None,
            emitted_at_ms: None,
            phase: "ready".to_string(),
            provider_requested_name: "OpenAI".to_string(),
            provider_name: "OpenAI".to_string(),
            provider_protocol: "openai".to_string(),
            provider_model: "gpt-5".to_string(),
            provider_source: "primary".to_string(),
            provider_mode: "standard".to_string(),
            fallback_reason: None,
            build_context_observation: None,
            input_tokens: None,
            cache_hit_input_tokens: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
            first_token_latency_ms: None,
            turn_duration_ms: None,
            user_message: "请继续处理".to_string(),
            assistant_message: "当前轮已收口。".to_string(),
            trace_steps: Vec::new(),
            trace_timeline: Vec::new(),
            tool_activities: Vec::new(),
            provider_call_records: Vec::new(),
            hook_trace_records: Vec::new(),
            session_summary: "summary".to_string(),
        };
        let checkpoint = ExecutionCheckpoint {
            contract_version: "execution-checkpoint-v1".to_string(),
            turn_id: "turn-graph".to_string(),
            session_id: Some("graph-session".to_string()),
            run_id: None,
            checkpoint_kind: "runtime_control".to_string(),
            recovery_mode: "replay_required".to_string(),
            projected_runtime_phase: "ready".to_string(),
            submission_command: None,
            resumable: false,
            replayable: false,
            status: "completed".to_string(),
            phase: "ready".to_string(),
            provider_requested_name: Some("OpenAI".to_string()),
            provider_name: Some("OpenAI".to_string()),
            provider_protocol: Some("openai".to_string()),
            provider_model: Some("gpt-5".to_string()),
            provider_source: Some("primary".to_string()),
            provider_mode: Some("standard".to_string()),
            fallback_reason: None,
            completed_hops: 0,
            max_hops: 16,
            active_tool_name: None,
            trace_steps: Vec::new(),
            tool_activities: Vec::new(),
            persisted_effect_evidence: Vec::new(),
            error: None,
            started_at_ms: 0,
            updated_at_ms: 0,
            stop_requested_at_ms: None,
        };

        let handoff = runtime.build_graph_turn_handoff(
            None,
            Some("turn-graph"),
            Some("graph-session"),
            &result,
            Some(&checkpoint),
        );
        let decision = runtime.decide_graph_after_turn(
            Some("turn-graph"),
            Some("graph-session"),
            &result,
            Some(&checkpoint),
        );

        assert_eq!(handoff.turn_id.as_deref(), Some("turn-graph"));
        assert_eq!(handoff.session_id.as_deref(), Some("graph-session"));
        assert_eq!(handoff.long_term_memory_status, "empty");
        assert_eq!(handoff.provider_name, "OpenAI");
        assert_eq!(
            decision.kind,
            crate::agent::graph::GraphDecisionKind::WaitUser
        );
    }

    #[test]
    fn run_turn_fails_when_attachment_persistence_fails() {
        let server = MockHttpServer::start(vec![json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "我看到了图片。"
                    }
                }
            ]
        }))]);
        let marker_path = temp_marker_file_path("pony-agent-attachment-failure");
        fs::write(&marker_path, "block attachment directory").expect("write marker file");
        let storage_path = marker_path.join("sessions.json");
        let sessions = SessionStore::with_backend(Box::new(FileSessionBackend::new(storage_path)));
        let mut selection = test_provider_selection(server.base_url.clone());
        selection.capabilities.supports_image_input = true;
        let mut runtime = build_runtime_with_session_store(selection, sessions);
        let result = runtime.run_turn(TurnInput {
            message: "请看这张图".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("attachment-failure".to_string()),
            node_id: None,
            history: Vec::new(),
            images: vec![TurnInputImage {
                data_url: "data:image/png;base64,AAAA".to_string(),
                mime_type: "image/png".to_string(),
                name: Some("diagram.png".to_string()),
            }],
        });

        assert_eq!(result.phase, "failed");
        assert!(result
            .assistant_message
            .contains("failed to create attachment directory"));
        let snapshot = runtime.load_session_snapshot(Some("attachment-failure"));
        assert!(snapshot.history.is_empty());

        let _ = server.finish();
        let _ = fs::remove_file(&marker_path);
    }

    #[test]
    fn recent_image_recall_requires_latest_user_turn_to_have_attachments() {
        let builder = DefaultTurnContextBuilder;
        let session = SessionSnapshot {
            conversation_id: "recall-session".to_string(),
            title: "新对话".to_string(),
            summary: "".to_string(),
            history: vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "[已附图片 1 张：old.png]".to_string(),
                    attachments: vec![SessionAttachment {
                        id: "att-old".to_string(),
                        asset_id: "asset-recall-session-att-old".to_string(),
                        name: Some("old.png".to_string()),
                        mime_type: "image/png".to_string(),
                        relative_path: "recall-session/att-old.dataurl".to_string(),
                        size_bytes: 4,
                        created_at_ms: 1,
                    }],
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "我看到了旧图。".to_string(),
                    attachments: Vec::new(),
                },
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "继续看 runtime.rs。".to_string(),
                    attachments: Vec::new(),
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "好的。".to_string(),
                    attachments: Vec::new(),
                },
            ],
            attachment_assets: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            history_state_audit_summary: crate::agent::session::HistoryStateAuditSummary::default(),
            run_control_audit_summary:
                crate::agent::session::build_missing_run_control_audit_summary(),
            turn_count: 2,
            last_referenced_file: None,
            updated_at_ms: 0,
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: Default::default(),
            resolved_node_id: None,
            latest_node_id: None,
        };

        let retrieved =
            builder.retrieve_context_state("那张图里有什么？", &[], &session, None, None);

        assert!(!should_recall_recent_images(&retrieved));
    }

    #[test]
    fn recent_image_recall_uses_retrieved_context_when_latest_user_turn_has_attachments() {
        let builder = DefaultTurnContextBuilder;
        let session = SessionSnapshot {
            conversation_id: "recall-session".to_string(),
            title: "新对话".to_string(),
            summary: "".to_string(),
            history: vec![
                TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "[已附图片 1 张：diagram.png]".to_string(),
                    attachments: vec![SessionAttachment {
                        id: "att-latest".to_string(),
                        asset_id: "asset-recall-session-att-latest".to_string(),
                        name: Some("diagram.png".to_string()),
                        mime_type: "image/png".to_string(),
                        relative_path: "recall-session/att-latest.dataurl".to_string(),
                        size_bytes: 4,
                        created_at_ms: 1,
                    }],
                },
                TurnHistoryMessage {
                    role: "assistant".to_string(),
                    content: "我看到了图。".to_string(),
                    attachments: Vec::new(),
                },
            ],
            attachment_assets: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            history_state_audit_summary: crate::agent::session::HistoryStateAuditSummary::default(),
            run_control_audit_summary:
                crate::agent::session::build_missing_run_control_audit_summary(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms: 0,
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: Default::default(),
            resolved_node_id: None,
            latest_node_id: None,
        };

        let retrieved =
            builder.retrieve_context_state("继续看这张图里有什么？", &[], &session, None, None);

        assert!(should_recall_recent_images(&retrieved));
    }

    #[test]
    fn run_turn_records_first_token_latency_for_reasoning_decision() {
        let server = MockHttpServer::start(vec![json_response(json!({
            "choices": [
                {
                    "message": {
                        "role": "assistant",
                        "content": "最终答案。",
                        "reasoning_content": "先想一下。"
                    }
                }
            ]
        }))]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let result = runtime.run_turn(TurnInput {
            message: "请直接回答。".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-reasoning-latency".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert_eq!(result.phase, "ready");
        assert_eq!(result.assistant_message, "最终答案。");
        assert!(result.first_token_latency_ms.is_some());

        let _ = server.finish();
    }

    #[test]
    fn run_turn_measures_first_token_latency_from_turn_start_across_tool_hops() {
        let final_text = "同步工具调用后返回最终答案。";
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": final_text
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 40,
                    "completion_tokens": 20,
                    "total_tokens": 60
                }
            })),
        ]);
        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection(server.base_url.clone()),
            Box::new(SlowToolExecutor { delay_ms: 40 }),
        );

        let result = runtime.run_turn(TurnInput {
            message: "先调用工具再同步回答".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-tool-hop-latency".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert_eq!(result.assistant_message, final_text);
        assert!(result.first_token_latency_ms.unwrap_or_default() >= 40);

        let _ = server.finish();
    }

    #[test]
    fn run_turn_completes_multi_hop_tool_followups_in_single_turn() {
        let final_text = "tauri.conf.json 的第 3 行是 `\"productName\": \"Pony Agent\",`。";
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            json_response(decision_tool_call(
                "workspace_read_file",
                json!({"path": "tauri.conf.json"}),
            )),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": final_text,
                            "reasoning_content": "已完成文件读取。"
                        }
                    }
                ]
            })),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let result = runtime.run_turn(TurnInput {
            message: "tauri.conf.json 第三行是什么？".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-multi-hop".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });
        let request_bodies = server.finish();

        assert_eq!(result.phase, "ready");
        assert_eq!(result.assistant_message, final_text);
        assert_eq!(result.tool_activities.len(), 2);
        assert_eq!(request_bodies.len(), 3);
        assert!(request_bodies[1].contains("\"tool_choice\":\"auto\""));
        assert!(request_bodies[2].contains("\"tool_choice\":\"auto\""));

        let snapshot = runtime.load_session_snapshot(Some("sync-multi-hop"));
        assert_eq!(snapshot.provider_native_transcript.len(), 6);
        assert_eq!(
            snapshot.provider_native_transcript[1]
                .get("tool_calls")
                .and_then(serde_json::Value::as_array)
                .map(|calls| calls.len()),
            Some(1)
        );
        assert_eq!(
            snapshot.provider_native_transcript[3]
                .get("tool_calls")
                .and_then(serde_json::Value::as_array)
                .map(|calls| calls.len()),
            Some(1)
        );
    }

    #[test]
    fn run_turn_accumulates_token_usage_across_tool_followups() {
        let final_text = "已累计整轮 token usage。";
        let server = MockHttpServer::start(vec![
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "先调用 workspace_list_files。",
                            "reasoning_content": "需要先执行 workspace_list_files。",
                            "tool_calls": [
                                {
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": json!({"path": ".", "limit": 40}).to_string()
                                    }
                                }
                            ]
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 100,
                    "prompt_cache_hit_tokens": 30,
                    "prompt_cache_miss_tokens": 70,
                    "completion_tokens": 20,
                    "total_tokens": 120,
                    "completion_tokens_details": {
                        "reasoning_tokens": 7
                    }
                }
            })),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "继续调用 workspace_read_file。",
                            "reasoning_content": "目录已找到，继续读取文件。",
                            "tool_calls": [
                                {
                                    "id": "call_workspace_read_file",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_read_file",
                                        "arguments": json!({"path": "tauri.conf.json"}).to_string()
                                    }
                                }
                            ]
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 80,
                    "prompt_cache_hit_tokens": 25,
                    "prompt_cache_miss_tokens": 55,
                    "completion_tokens": 10,
                    "total_tokens": 90,
                    "completion_tokens_details": {
                        "reasoning_tokens": 3
                    }
                }
            })),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": final_text,
                            "reasoning_content": "已经整理完最终结果。"
                        }
                    }
                ],
                "usage": {
                    "prompt_tokens": 60,
                    "prompt_cache_hit_tokens": 15,
                    "prompt_cache_miss_tokens": 45,
                    "completion_tokens": 40,
                    "total_tokens": 100,
                    "completion_tokens_details": {
                        "reasoning_tokens": 2
                    }
                }
            })),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let result = runtime.run_turn(TurnInput {
            message: "继续读取 tauri.conf.json 第三行".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("sync-usage-accumulated".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert_eq!(result.phase, "ready");
        assert_eq!(result.assistant_message, final_text);
        assert_eq!(result.input_tokens, Some(240));
        assert_eq!(result.cache_hit_input_tokens, Some(70));
        assert_eq!(result.reasoning_tokens, Some(12));
        assert_eq!(result.output_tokens, Some(70));
        assert_eq!(result.total_tokens, Some(310));
        let snapshot = runtime.load_session_snapshot(Some("sync-usage-accumulated"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("sync accumulated trace");
        assert_eq!(trace.provider_call_records.len(), 3);
        assert_eq!(
            trace.provider_call_records[0].request_kind,
            ProviderRequestKind::InitialRequest
        );
        assert_eq!(
            trace.provider_call_records[1].request_kind,
            ProviderRequestKind::ToolFollowup
        );
        assert_eq!(
            trace.provider_call_records[2].cache_miss_input_tokens,
            Some(45)
        );

        let _ = server.finish();
    }

    #[test]
    fn run_turn_repairs_blank_tool_name_before_execution() {
        let final_text = "tauri.conf.json 已成功读取。";
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            json_response(decision_blank_tool_call(json!({
                "path": "tauri.conf.json"
            }))),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": final_text,
                            "reasoning_content": "空工具名已修复并继续执行。"
                        }
                    }
                ]
            })),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let result = runtime.run_turn(TurnInput {
            message: "继续查看 tauri.conf.json".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("repair-blank-tool-sync".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });

        assert_eq!(result.phase, "ready");
        assert_eq!(result.assistant_message, final_text);
        assert_eq!(result.tool_activities.len(), 2);
        assert!(result
            .tool_activities
            .iter()
            .any(|activity| activity.name == "workspace_read_file"));

        let snapshot = runtime.load_session_snapshot(Some("repair-blank-tool-sync"));
        assert_eq!(
            snapshot.provider_native_transcript[3]
                .get("tool_calls")
                .and_then(serde_json::Value::as_array)
                .and_then(|calls| calls.first())
                .and_then(|call| call.get("function"))
                .and_then(|function| function.get("name"))
                .and_then(serde_json::Value::as_str),
            Some("workspace_read_file")
        );

        let _ = server.finish();
    }

    #[test]
    fn tool_hop_limit_uses_default_when_env_is_missing() {
        assert_eq!(
            parse_max_tool_hops_per_turn(None),
            DEFAULT_MAX_TOOL_HOPS_PER_TURN
        );
    }

    #[test]
    fn tool_hop_limit_accepts_reasonable_env_override() {
        assert_eq!(parse_max_tool_hops_per_turn(Some("24")), 24);
        assert_eq!(parse_max_tool_hops_per_turn(Some("1000")), 1000);
    }

    #[test]
    fn tool_hop_limit_rejects_invalid_env_values() {
        assert_eq!(
            parse_max_tool_hops_per_turn(Some("0")),
            DEFAULT_MAX_TOOL_HOPS_PER_TURN
        );
        assert_eq!(
            parse_max_tool_hops_per_turn(Some("5000")),
            DEFAULT_MAX_TOOL_HOPS_PER_TURN
        );
        assert_eq!(
            parse_max_tool_hops_per_turn(Some("not-a-number")),
            DEFAULT_MAX_TOOL_HOPS_PER_TURN
        );
    }

    #[test]
    fn tool_followup_limit_uses_default_when_env_is_missing() {
        assert_eq!(
            parse_max_tool_followups_per_turn(None),
            DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN
        );
    }

    #[test]
    fn tool_followup_limit_accepts_reasonable_env_override() {
        assert_eq!(parse_max_tool_followups_per_turn(Some("3")), 3);
        assert_eq!(parse_max_tool_followups_per_turn(Some("12")), 12);
    }

    #[test]
    fn tool_followup_limit_rejects_invalid_env_values() {
        assert_eq!(
            parse_max_tool_followups_per_turn(Some("0")),
            DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN
        );
        assert_eq!(
            parse_max_tool_followups_per_turn(Some("64")),
            DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN
        );
        assert_eq!(
            parse_max_tool_followups_per_turn(Some("not-a-number")),
            DEFAULT_MAX_TOOL_FOLLOWUPS_PER_TURN
        );
    }

    #[test]
    fn tool_call_signature_normalizes_object_key_order() {
        let left = ToolCall {
            call_id: None,
            name: "workspace_list_files".to_string(),
            arguments: json!({"path":"src/agent","limit":20}),
            plan: None,
        };
        let right = ToolCall {
            call_id: None,
            name: "workspace_list_files".to_string(),
            arguments: json!({"limit":20,"path":"src/agent"}),
            plan: None,
        };

        assert_eq!(tool_call_signature(&left), tool_call_signature(&right));
    }

    #[test]
    fn stream_reasoning_batcher_batches_until_threshold() {
        let mut batcher = StreamReasoningBatcher::default();
        assert!(batcher.push("abc".repeat(10)).is_none());
        let chunk = batcher
            .push("d".repeat(STREAM_REASONING_BATCH_CHARS))
            .unwrap();
        assert!(chunk.chars().count() >= STREAM_REASONING_BATCH_CHARS);
        assert!(batcher.flush().is_none());
    }

    #[test]
    fn start_turn_stream_completes_after_multi_hop_followup_stream() {
        let final_text = "tauri.conf.json 的第 3 行是 `\"productName\": \"Pony Agent\",`。";
        let server = MockHttpServer::start(vec![
            sse_response(&[json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "需要先执行 workspace_list_files。",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": "{\"path\":\".\",\"limit\":40}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            })]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "已经找到 tauri.conf.json。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "找到了！让我读取 tauri.conf.json 的内容："
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_read_file",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_read_file",
                                            "arguments": "{\"path\":\"tauri"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "function": {
                                            "arguments": ".conf.json\"}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "已读取目标文件。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
            ]),
        ]);
        let mut runtime = AgentRuntime::with_dependencies(
            SessionStore::memory_only(),
            Box::new(StaticResolver {
                selection: test_provider_selection(server.base_url.clone()),
            }),
            Box::new(StubToolExecutor),
            Box::new(SlowPassthroughPlanner { delay_ms: 40 }),
            Box::new(DefaultTurnContextBuilder),
            Box::new(DefaultTurnTelemetryBuilder),
        );
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.model",
                10,
                TurnHookPoint::ModelCallStart,
            ))
            .expect("register model hook");
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.tool-start",
                20,
                TurnHookPoint::ToolCallStart,
            ))
            .expect("register tool-start hook");
        runtime
            .register_hook_descriptor(observe_hook_descriptor(
                "observe.tool-end",
                30,
                TurnHookPoint::ToolCallEnd,
            ))
            .expect("register tool-end hook");
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-multi-hop".to_string(),
            TurnInput {
                message: "继续读取 tauri.conf.json 第三行".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-multi-hop".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );
        let request_bodies = server.finish();
        let events = sink.events.borrow();
        let completed = events
            .iter()
            .find_map(|(name, payload)| {
                if name == "turn:completed" {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .expect("completed event");
        let tool_completed = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:tool"
                    && payload.event_type.as_deref() == Some("turn.tool_call_completed"))
                .then_some(payload.clone())
            })
            .expect("tool completed event");
        let checkpoint_phase_changed = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:phase_changed"
                    && payload.event_type.as_deref() == Some("turn.phase_changed")
                    && payload.phase.as_deref() == Some("checkpointing"))
                .then_some(payload.clone())
            })
            .expect("checkpoint phase changed event");
        let checkpoint_persisted = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:checkpoint_persisted"
                    && payload.event_type.as_deref() == Some("turn.checkpoint_persisted"))
                .then_some(payload.clone())
            })
            .expect("checkpoint persisted event");
        let tool_started = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:tool"
                    && payload.event_type.as_deref() == Some("turn.tool_call_started"))
                .then_some(payload.clone())
            })
            .expect("tool started event");
        let model_started_events: Vec<TurnStreamEvent> = events
            .iter()
            .filter_map(|(name, payload)| {
                (name == "turn:trace"
                    && payload.event_type.as_deref() == Some("turn.model_call_started"))
                .then_some(payload.clone())
            })
            .collect();

        assert_eq!(completed.text.as_deref(), Some(final_text));
        assert!(model_started_events.len() >= 2);
        assert!(model_started_events.iter().all(|payload| {
            payload
                .hook_trace_records
                .as_ref()
                .map(|records| {
                    records.iter().any(|record| {
                        record.hook_name == "observe.model"
                            && record.hook_point == TurnHookPoint::ModelCallStart
                    })
                })
                .unwrap_or(false)
        }));
        assert_hook_boundary_alignment(
            &tool_started,
            TurnHookPoint::ToolCallStart,
            "turn.tool_call_started",
            "executing_tool",
        );
        assert_eq!(
            tool_started
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert_eq!(
            tool_started
                .hook_trace_records
                .as_ref()
                .and_then(|records| records.first())
                .map(|record| record.hook_name.as_str()),
            Some("observe.tool-start")
        );
        assert_hook_boundary_alignment(
            &checkpoint_phase_changed,
            TurnHookPoint::CheckpointPersistStart,
            "turn.phase_changed",
            "checkpointing",
        );
        assert_hook_boundary_alignment(
            &checkpoint_persisted,
            TurnHookPoint::CheckpointPersistEnd,
            "turn.checkpoint_persisted",
            "checkpointing",
        );
        assert_hook_boundary_alignment(
            &completed,
            TurnHookPoint::TurnFinalizeEnd,
            "turn.completed",
            "completed",
        );
        assert_hook_boundary_alignment(
            &tool_completed,
            TurnHookPoint::ToolCallEnd,
            "turn.tool_call_completed",
            "tool_result_integrating",
        );
        assert_eq!(
            tool_completed
                .hook_trace_records
                .as_ref()
                .map(|records| records.len()),
            Some(1)
        );
        assert_eq!(
            tool_completed
                .hook_trace_records
                .as_ref()
                .and_then(|records| records.first())
                .map(|record| record.hook_name.as_str()),
            Some("observe.tool-end")
        );
        assert!(
            checkpoint_phase_changed.sequence.unwrap_or_default()
                < checkpoint_persisted.sequence.unwrap_or_default()
        );
        assert!(
            checkpoint_persisted.sequence.unwrap_or_default()
                < completed.sequence.unwrap_or_default()
        );
        assert_eq!(
            completed
                .trace_timeline
                .as_ref()
                .and_then(|timeline| timeline.last())
                .map(|entry| entry.kind.as_str()),
            Some("checkpoint_persist")
        );
        assert_eq!(
            completed
                .tool_activities
                .as_ref()
                .map(|activities| activities.len()),
            Some(2)
        );
        assert!(events.iter().any(|(name, payload)| {
            name == "turn:tool"
                && payload
                    .tool_activities
                    .as_ref()
                    .map(|activities| {
                        activities
                            .iter()
                            .any(|activity| activity.name == "workspace_read_file")
                    })
                    .unwrap_or(false)
        }));
        assert_eq!(request_bodies.len(), 3);
        assert!(request_bodies[1].contains("\"tool_choice\":\"auto\""));
        assert!(request_bodies[2].contains("\"tool_choice\":\"auto\""));
    }

    #[test]
    fn start_turn_stream_fails_with_canonical_finalize_boundary_when_tool_hop_limit_is_hit() {
        let _limit_guard = ToolHopLimitOverrideGuard::set(1);
        let server = MockHttpServer::start(vec![
            json_response(decision_tool_call(
                "workspace_list_files",
                json!({"path": ".", "limit": 40}),
            )),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "已经找到目标文件。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_read_file",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_read_file",
                                            "arguments": "{\"path\":\"tauri.conf.json\"}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-tool-hop-limit".to_string(),
            TurnInput {
                message: "请连续调用两个工具".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-tool-hop-limit".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let request_bodies = server.finish();
        let events = sink.events.borrow();
        let failed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
            .expect("failed event");
        let tool_completed = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:tool"
                    && payload.event_type.as_deref() == Some("turn.tool_call_completed"))
                .then_some(payload.clone())
            })
            .expect("tool completed event before hop-limit failure");
        let expected_error = build_tool_hop_limit_error(1);

        assert_hook_boundary_alignment(
            &failed,
            TurnHookPoint::TurnFinalizeEnd,
            "turn.failed",
            "failed",
        );
        assert_hook_boundary_alignment(
            &tool_completed,
            TurnHookPoint::ToolCallEnd,
            "turn.tool_call_completed",
            "tool_result_integrating",
        );
        assert_eq!(failed.error.as_deref(), Some(expected_error.as_str()));
        assert_eq!(
            failed
                .tool_activities
                .as_ref()
                .map(|activities| activities.len()),
            Some(1)
        );
        assert_eq!(request_bodies.len(), 2);

        let snapshot = runtime.load_session_snapshot(Some("stream-tool-hop-limit"));
        let persisted_trace = snapshot
            .turn_trace_history
            .last()
            .expect("failed trace should be persisted");
        assert_eq!(persisted_trace.phase, "failed");
        assert_eq!(
            persisted_trace.error.as_deref(),
            Some(expected_error.as_str())
        );
        assert_eq!(persisted_trace.tool_activities.len(), 1);
    }

    #[test]
    fn start_turn_stream_cancels_with_canonical_finalize_boundary_when_stop_is_requested_during_tool_execution(
    ) {
        let server = MockHttpServer::start(vec![json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        ))]);
        let control = Arc::new(ExecutionControlRegistry::new());
        let turn_id = "turn-cancel-during-tool".to_string();
        control.register_turn(&turn_id, Some("stream-cancel-during-tool"), None);

        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection(server.base_url.clone()),
            Box::new(RequestStopToolExecutor {
                control: Arc::clone(&control),
                turn_id: turn_id.clone(),
                delay_ms: 30,
            }),
        );
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream_with_control(
            &sink,
            &control,
            turn_id.clone(),
            TurnInput {
                message: "先调用工具，然后我会中止".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-cancel-during-tool".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let request_bodies = server.finish();
        let events = sink.events.borrow();
        let cancelled = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:cancelled").then_some(payload.clone()))
            .expect("cancelled event");
        let tool_started = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:tool"
                    && payload.event_type.as_deref() == Some("turn.tool_call_started"))
                .then_some(payload.clone())
            })
            .expect("tool started event");

        assert_hook_boundary_alignment(
            &tool_started,
            TurnHookPoint::ToolCallStart,
            "turn.tool_call_started",
            "executing_tool",
        );
        assert_hook_boundary_alignment(
            &cancelled,
            TurnHookPoint::TurnFinalizeEnd,
            "turn.cancelled",
            "cancelled",
        );
        assert_eq!(cancelled.error.as_deref(), Some("stopped_by_user"));
        assert_eq!(
            cancelled
                .tool_activities
                .as_ref()
                .map(|activities| activities.len()),
            Some(1)
        );
        assert!(!events.iter().any(|(name, payload)| {
            name == "turn:tool" && payload.event_type.as_deref() == Some("turn.tool_call_completed")
        }));
        assert_eq!(request_bodies.len(), 1);

        let snapshot = runtime.load_session_snapshot(Some("stream-cancel-during-tool"));
        assert_eq!(
            snapshot
                .history
                .last()
                .map(|message| message.content.as_str()),
            Some(CANCELLED_TURN_MESSAGE)
        );
        let persisted_trace = snapshot
            .turn_trace_history
            .last()
            .expect("cancelled trace should be persisted");
        assert_eq!(persisted_trace.phase, "cancelled");
        assert_eq!(persisted_trace.error.as_deref(), Some("stopped_by_user"));
        assert_eq!(persisted_trace.tool_activities.len(), 1);
    }

    #[test]
    fn start_turn_stream_fails_with_canonical_finalize_boundary_when_tool_execution_errors() {
        let server = MockHttpServer::start(vec![json_response(decision_tool_call(
            "workspace_list_files",
            json!({"path": ".", "limit": 40}),
        ))]);
        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection(server.base_url.clone()),
            Box::new(ErrorToolExecutor),
        );
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-tool-error".to_string(),
            TurnInput {
                message: "调用工具但让它失败".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-tool-error".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let request_bodies = server.finish();
        let events = sink.events.borrow();
        let tool_started = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:tool"
                    && payload.event_type.as_deref() == Some("turn.tool_call_started"))
                .then_some(payload.clone())
            })
            .expect("tool started event");
        let tool_completed = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:tool"
                    && payload.event_type.as_deref() == Some("turn.tool_call_completed"))
                .then_some(payload.clone())
            })
            .expect("tool completed event");
        let failed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:failed").then_some(payload.clone()))
            .expect("failed event");
        let expected_error = build_tool_execution_error(
            "workspace_list_files",
            "tool workspace_list_files failed in test",
        );

        assert_hook_boundary_alignment(
            &tool_started,
            TurnHookPoint::ToolCallStart,
            "turn.tool_call_started",
            "executing_tool",
        );
        assert_hook_boundary_alignment(
            &tool_completed,
            TurnHookPoint::ToolCallEnd,
            "turn.tool_call_completed",
            "tool_result_integrating",
        );
        assert_hook_boundary_alignment(
            &failed,
            TurnHookPoint::TurnFinalizeEnd,
            "turn.failed",
            "failed",
        );
        assert_eq!(failed.error.as_deref(), Some(expected_error.as_str()));
        assert_eq!(
            failed
                .tool_activities
                .as_ref()
                .map(|activities| activities.len()),
            Some(1)
        );
        assert!(!events.iter().any(|(name, _)| name == "turn:completed"));
        assert_eq!(request_bodies.len(), 1);

        let snapshot = runtime.load_session_snapshot(Some("stream-tool-error"));
        let persisted_trace = snapshot
            .turn_trace_history
            .last()
            .expect("failed trace should be persisted");
        assert_eq!(persisted_trace.phase, "failed");
        assert_eq!(
            persisted_trace.error.as_deref(),
            Some(expected_error.as_str())
        );
        assert_eq!(persisted_trace.tool_activities.len(), 1);
    }

    #[test]
    fn start_turn_stream_emits_first_token_latency_on_reasoning_delta() {
        let server = MockHttpServer::start(vec![sse_response(&[
            json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "先想一下。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [
                    {
                        "delta": {
                            "content": "最终答案。"
                        }
                    }
                ]
            }),
            json!({
                "choices": [],
                "usage": {
                    "prompt_tokens": 20,
                    "completion_tokens": 6,
                    "total_tokens": 26
                }
            }),
        ])]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-reasoning-latency".to_string(),
            TurnInput {
                message: "请直接回答。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-reasoning-latency".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let first_delta = events
            .iter()
            .find_map(|(name, payload)| {
                if name == "turn:delta" {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .expect("first delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| {
                if name == "turn:completed" {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .expect("completed event");
        let checkpoint_phase_changed = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:phase_changed"
                    && payload.event_type.as_deref() == Some("turn.phase_changed")
                    && payload.phase.as_deref() == Some("checkpointing"))
                .then_some(payload.clone())
            })
            .expect("checkpoint phase changed event");
        let checkpoint_persisted = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:checkpoint_persisted"
                    && payload.event_type.as_deref() == Some("turn.checkpoint_persisted"))
                .then_some(payload.clone())
            })
            .expect("checkpoint persisted event");

        assert_eq!(first_delta.reasoning_content.as_deref(), Some("先想一下。"));
        assert_eq!(first_delta.text, None);
        assert!(first_delta.first_token_latency_ms.is_some());
        assert_eq!(
            completed.first_token_latency_ms,
            first_delta.first_token_latency_ms
        );
        assert_hook_boundary_alignment(
            &checkpoint_phase_changed,
            TurnHookPoint::CheckpointPersistStart,
            "turn.phase_changed",
            "checkpointing",
        );
        assert_hook_boundary_alignment(
            &checkpoint_persisted,
            TurnHookPoint::CheckpointPersistEnd,
            "turn.checkpoint_persisted",
            "checkpointing",
        );
        assert!(
            checkpoint_persisted.sequence.unwrap_or_default()
                < completed.sequence.unwrap_or_default()
        );
        assert_eq!(
            completed
                .trace_timeline
                .as_ref()
                .and_then(|timeline| timeline.last())
                .map(|entry| entry.kind.as_str()),
            Some("checkpoint_persist")
        );
        let decision_request: Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        assert_eq!(
            decision_request.get("stream").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_decision_stream")
        );
        let provider_call = completed
            .provider_call_records
            .as_ref()
            .and_then(|records| records.first())
            .expect("initial provider call record");
        assert_eq!(
            provider_call.latency_kind,
            ProviderLatencyKind::ProviderStream
        );
        assert!(provider_call.first_token_latency_ms.is_some());
        assert!(provider_call.turn_duration_ms.is_some());
        assert!(
            provider_call.first_token_latency_ms.unwrap()
                <= provider_call.turn_duration_ms.unwrap()
        );
        assert!(
            completed.first_token_latency_ms.unwrap()
                > provider_call.first_token_latency_ms.unwrap()
        );
    }

    #[test]
    fn start_turn_stream_sync_fallback_for_initial_decision_does_not_emit_fake_ttft() {
        let server = MockHttpServer::start(vec![
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "最终答案。",
                            "reasoning_content": "先想一下。"
                        }
                    }
                ]
            })),
            json_response(json!({
                "choices": [
                    {
                        "message": {
                            "role": "assistant",
                            "content": "最终答案。",
                            "reasoning_content": "先想一下。"
                        }
                    }
                ]
            })),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-sync-fallback-no-fake-ttft".to_string(),
            TurnInput {
                message: "请直接回答。".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-sync-fallback-no-fake-ttft".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let first_delta = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .expect("first delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(first_delta.first_token_latency_ms, None);
        assert_eq!(completed.first_token_latency_ms, None);
        let streamed_decision_request: Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        let fallback_decision_request: Value =
            serde_json::from_str(&requests[1]).expect("fallback decision request should be json");
        assert_eq!(
            streamed_decision_request
                .get("stream")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            fallback_decision_request
                .get("stream")
                .and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_decision")
        );
    }

    #[test]
    fn start_turn_stream_measures_first_token_latency_from_turn_start_across_tool_hops() {
        let final_text = "工具调用后返回最终答案。";
        let server = MockHttpServer::start(vec![
            sse_response(&[json!({
                "choices": [
                    {
                        "delta": {
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": "{\"path\":\".\",\"limit\":40}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            })]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 40,
                        "completion_tokens": 20,
                        "total_tokens": 60
                    }
                }),
            ]),
        ]);
        let mut runtime = build_runtime_for_test_with_tool_executor(
            test_provider_selection(server.base_url.clone()),
            Box::new(SlowToolExecutor { delay_ms: 40 }),
        );
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-tool-hop-latency".to_string(),
            TurnInput {
                message: "先调用工具再回答".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-tool-hop-latency".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let _ = server.finish();
        let events = sink.events.borrow();
        let first_delta = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .expect("first delta event");
        let tool_completed = events
            .iter()
            .find_map(|(name, payload)| {
                (name == "turn:tool"
                    && payload.event_type.as_deref() == Some("turn.tool_call_completed"))
                .then_some(payload.clone())
            })
            .expect("tool completed event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert!(first_delta.first_token_latency_ms.unwrap_or_default() >= 40);
        assert_eq!(
            completed.first_token_latency_ms,
            first_delta.first_token_latency_ms
        );
        assert_hook_boundary_alignment(
            &tool_completed,
            TurnHookPoint::ToolCallEnd,
            "turn.tool_call_completed",
            "tool_result_integrating",
        );
        assert_hook_boundary_alignment(
            &completed,
            TurnHookPoint::TurnFinalizeEnd,
            "turn.completed",
            "completed",
        );
    }

    #[test]
    fn start_turn_stream_uses_live_stream_for_deepseek_tool_followup() {
        let final_text = "deepseek follow-up completed";
        let server = MockHttpServer::start(vec![
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "need workspace listing before answering"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "call a tool first",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_list_files",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_list_files",
                                            "arguments": "{\"path\":\".\",\"limit\":40}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "tool output is sufficient"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 60,
                        "completion_tokens": 24,
                        "total_tokens": 84
                    }
                }),
            ]),
        ]);
        let mut runtime =
            build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-deepseek-followup-compat".to_string(),
            TurnInput {
                message: "read Cargo.toml then answer".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("deepseek-followup-compat".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let text_delta = events
            .iter()
            .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .find(|payload| payload.text.as_deref() == Some(final_text))
            .expect("text delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(requests.len(), 2);
        let decision_request: Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        let followup_request: Value =
            serde_json::from_str(&requests[1]).expect("followup request should be json");
        assert_eq!(
            decision_request.get("stream").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            followup_request.get("stream").and_then(Value::as_bool),
            Some(true)
        );
        assert!(followup_request.get("stream_options").is_some());
        let replayed_assistant_message = followup_request
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages.iter().find(|message| {
                    message.get("role").and_then(Value::as_str) == Some("assistant")
                        && message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(|calls| !calls.is_empty())
                            .unwrap_or(false)
                })
            })
            .expect("follow-up request should replay assistant tool call message");
        assert_eq!(
            replayed_assistant_message
                .get("reasoning_content")
                .and_then(Value::as_str),
            Some("need workspace listing before answering")
        );
        assert_eq!(text_delta.text.as_deref(), Some(final_text));
        assert_eq!(completed.phase.as_deref(), Some("completed"));
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_followup_stream")
        );
        assert_eq!(completed.fallback_reason, None);
        let provider_calls = completed
            .provider_call_records
            .as_ref()
            .expect("provider call records");
        assert_eq!(provider_calls.len(), 2);
        assert!(provider_calls
            .iter()
            .all(|record| record.latency_kind == ProviderLatencyKind::ProviderStream));
        assert!(provider_calls
            .iter()
            .all(|record| record.first_token_latency_ms.is_some()));
    }

    #[test]
    fn start_turn_stream_streams_duplicate_tool_recovery_answer() {
        let final_text = "根据已读取的内容，tauri.conf.json 中 productName 是 Pony Agent。";
        let duplicate_args = "{\"path\":\"tauri.conf.json\"}";
        let server = MockHttpServer::start(vec![
            sse_response(&[json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "先读取配置文件。",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_read_file",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_read_file",
                                        "arguments": duplicate_args
                                    }
                                }
                            ]
                        }
                    }
                ]
            })]),
            sse_response(&[json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "还想重复读取同一个文件。",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_read_file_again",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_read_file",
                                        "arguments": duplicate_args
                                    }
                                }
                            ]
                        }
                    }
                ]
            })]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "重复工具调用已停止，直接总结。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
            ]),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-duplicate-tool-recovery-stream".to_string(),
            TurnInput {
                message: "读取 tauri.conf.json 并回答 productName".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-duplicate-tool-recovery".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let text_delta = events
            .iter()
            .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .find(|payload| payload.text.as_deref() == Some(final_text))
            .expect("recovery answer should be streamed as delta");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(requests.len(), 3);
        assert!(text_delta.trace_timeline.is_none());
        assert_eq!(completed.text.as_deref(), Some(final_text));
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_followup_stream")
        );
        let provider_calls = completed
            .provider_call_records
            .as_ref()
            .expect("provider call records");
        assert_eq!(
            provider_calls.last().map(|record| &record.latency_kind),
            Some(&ProviderLatencyKind::ProviderStream)
        );
        assert!(provider_calls
            .last()
            .and_then(|record| record.first_token_latency_ms)
            .is_some());
    }

    #[test]
    fn start_turn_stream_accumulates_token_usage_across_tool_followups() {
        let final_text = "流式回合已累计整轮 token usage。";
        let server = MockHttpServer::start(vec![
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "需要先执行 workspace_list_files。",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_list_files",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_list_files",
                                            "arguments": "{\"path\":\".\",\"limit\":40}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 100,
                        "prompt_cache_hit_tokens": 30,
                        "prompt_cache_miss_tokens": 70,
                        "completion_tokens": 20,
                        "total_tokens": 120,
                        "completion_tokens_details": {
                            "reasoning_tokens": 7
                        }
                    }
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "目录已找到。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "继续读取 tauri.conf.json。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_read_file",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_read_file",
                                            "arguments": "{\"path\":\"tauri.conf.json\"}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 80,
                        "prompt_cache_hit_tokens": 25,
                        "prompt_cache_miss_tokens": 55,
                        "completion_tokens": 10,
                        "total_tokens": 90,
                        "completion_tokens_details": {
                            "reasoning_tokens": 3
                        }
                    }
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "已经整理完最终结果。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 60,
                        "prompt_cache_hit_tokens": 15,
                        "prompt_cache_miss_tokens": 45,
                        "completion_tokens": 40,
                        "total_tokens": 100,
                        "completion_tokens_details": {
                            "reasoning_tokens": 2
                        }
                    }
                }),
            ]),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-usage-accumulated".to_string(),
            TurnInput {
                message: "继续读取 tauri.conf.json 第三行".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("stream-usage-accumulated".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let events = sink.events.borrow();
        let completed = events
            .iter()
            .find_map(|(name, payload)| {
                if name == "turn:completed" {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .expect("stream completed event");

        assert_eq!(completed.text.as_deref(), Some(final_text));
        assert_eq!(completed.input_tokens, Some(240));
        assert_eq!(completed.cache_hit_input_tokens, Some(70));
        assert_eq!(completed.reasoning_tokens, Some(12));
        assert_eq!(completed.output_tokens, Some(70));
        assert_eq!(completed.total_tokens, Some(310));

        let snapshot = runtime.load_session_snapshot(Some("stream-usage-accumulated"));
        let trace = snapshot
            .turn_trace_history
            .last()
            .expect("stream accumulated trace");
        assert_eq!(trace.input_tokens, Some(240));
        assert_eq!(trace.cache_hit_input_tokens, Some(70));
        assert_eq!(trace.reasoning_tokens, Some(12));
        assert_eq!(trace.output_tokens, Some(70));
        assert_eq!(trace.total_tokens, Some(310));
        assert_eq!(trace.provider_call_records.len(), 3);
        assert_eq!(
            trace.provider_call_records[0].request_kind,
            ProviderRequestKind::InitialRequest
        );
        assert_eq!(
            trace.provider_call_records[1].request_kind,
            ProviderRequestKind::ToolFollowup
        );
        assert_eq!(
            trace.provider_call_records[2].cache_miss_input_tokens,
            Some(45)
        );

        let _ = server.finish();
    }

    #[test]
    fn start_turn_stream_repairs_blank_tool_name_in_followup_stream() {
        let final_text = "tauri.conf.json 已在流式回合中成功读取。";
        let server = MockHttpServer::start(vec![
            sse_response(&[json!({
                "choices": [
                    {
                        "delta": {
                            "reasoning_content": "需要先执行 workspace_list_files。",
                            "tool_calls": [
                                {
                                    "index": 0,
                                    "id": "call_workspace_list_files",
                                    "type": "function",
                                    "function": {
                                        "name": "workspace_list_files",
                                        "arguments": "{\"path\":\".\",\"limit\":40}"
                                    }
                                }
                            ]
                        }
                    }
                ]
            })]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "继续读取文件内容。"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_blank_name_stream",
                                        "type": "function",
                                        "function": {
                                            "name": "",
                                            "arguments": "{\"path\":\"tauri.conf.json\"}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[json!({
                "choices": [
                    {
                        "delta": {
                            "content": final_text
                        }
                    }
                ]
            })]),
        ]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-repair-blank-stream".to_string(),
            TurnInput {
                message: "继续读取 tauri.conf.json".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("repair-blank-tool-stream".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let request_bodies = server.finish();
        let events = sink.events.borrow();
        let completed = events
            .iter()
            .find_map(|(name, payload)| {
                if name == "turn:completed" {
                    Some(payload.clone())
                } else {
                    None
                }
            })
            .expect("completed event");

        assert_eq!(completed.text.as_deref(), Some(final_text));
        assert!(events.iter().any(|(name, payload)| {
            name == "turn:tool"
                && payload
                    .tool_activities
                    .as_ref()
                    .map(|activities| {
                        activities
                            .iter()
                            .any(|activity| activity.name == "workspace_read_file")
                    })
                    .unwrap_or(false)
        }));
        assert_eq!(request_bodies.len(), 3);
    }

    #[test]
    fn runtime_can_rebuild_session_snapshot_and_retrieved_context_from_history_node() {
        let server =
            MockHttpServer::start(vec![json_completion("第一答"), json_completion("第二答")]);
        let mut runtime = build_runtime_for_test(test_provider_selection(server.base_url.clone()));

        let first = runtime.run_turn(TurnInput {
            message: "第一问".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("runtime-history".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });
        let second = runtime.run_turn(TurnInput {
            message: "第二问".to_string(),
            display_message: None,
            provider_id: None,
            model_id: None,
            reasoning_effort: None,
            session_id: Some("runtime-history".to_string()),
            node_id: None,
            history: Vec::new(),
            images: Vec::new(),
        });
        assert_eq!(first.assistant_message, "第一答");
        assert_eq!(second.assistant_message, "第二答");

        let (history_nodes, _, _) = runtime.load_history_graph(Some("runtime-history"));
        let historical_node_id = history_nodes
            .first()
            .map(|node| node.node_id.clone())
            .expect("historical node should exist");

        let snapshot = runtime
            .load_session_snapshot_at(Some("runtime-history"), Some(historical_node_id.as_str()));
        assert_eq!(
            snapshot.resolved_node_id.as_deref(),
            Some(historical_node_id.as_str())
        );
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].content, "第一问");
        assert_eq!(
            snapshot.history_cursor.mode,
            crate::agent::session::HistoryCursorMode::Historical
        );

        let retrieved = runtime.inspect_retrieved_context_at(
            Some("runtime-history"),
            Some(historical_node_id.as_str()),
            None,
            None,
        );
        assert_eq!(retrieved.session_context.turn_count, 1);
        assert_eq!(retrieved.session_context.recent_history.len(), 2);

        let _ = server.finish();
    }

    #[test]
    fn persisted_trace_timeline_uses_canonical_monitor_semantics() {
        let provider_meta = ProviderEventMeta {
            requested_name: "OpenAI".to_string(),
            provider_name: "OpenAI".to_string(),
            protocol: "openai".to_string(),
            model: "gpt-5".to_string(),
        };
        let build_context_observation = BuildContextObservation {
            request_format: "responses".to_string(),
            message_count: 4,
            image_count: 0,
            tool_count: 1,
            temperature: 0.0,
            max_output_tokens: 1024,
            stable_prefix_text: "system: stable".to_string(),
            semi_stable_context_text: "developer: retrieval summary".to_string(),
            volatile_input_text: "user: request".to_string(),
            prefix_mutation_reasons: vec![
                crate::agent::provider::PrefixMutationReason::HistoryBoundaryShifted,
            ],
            request_messages_text: "system: stable\nuser: request".to_string(),
            tool_definitions_text: "workspace.read_file(path)".to_string(),
        };
        let tool_activities = vec![crate::agent::telemetry::TurnToolActivity {
            id: "tool-read-file".to_string(),
            name: "workspace.read_file".to_string(),
            status: "done".to_string(),
            summary: "read file done".to_string(),
            arguments_text: Some("{\"path\":\"src/main.ts\"}".to_string()),
            result_text: Some("{\"content\":\"ok\"}".to_string()),
            duration_seconds: Some(0.2),
            capability_invocation: None,
        }];

        let timeline = build_persisted_trace_timeline(
            "读取文件",
            "completed",
            Some(&provider_meta),
            Some("primary"),
            Some("standard"),
            Some(&build_context_observation),
            &tool_activities,
            Some("final answer"),
            None,
            None,
            None,
            Some(11),
            Some(3),
            Some(0),
            Some(7),
            Some(18),
            Some(99),
            Some(900),
        );

        let kinds = timeline
            .iter()
            .map(|entry| entry.kind.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            kinds,
            vec![
                "input",
                "prepare_retrieval",
                "build_context",
                "call_model",
                "call_tool",
                "call_model",
            ]
        );
        assert_eq!(timeline[1].label, "PREPARE RETRIEVAL");
        assert_eq!(timeline[4].label, "CALL TOOL #1 · workspace.read_file");
        assert_eq!(timeline[5].label, "CALL MODEL #2");
    }

    #[test]
    fn deepseek_tool_followup_uses_live_stream() {
        let final_text = "deepseek follow-up completed";
        let server = MockHttpServer::start(vec![
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "need workspace listing before answering"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "call a tool first",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_list_files",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_list_files",
                                            "arguments": "{\"path\":\".\",\"limit\":40}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": "tool output is sufficient"
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 60,
                        "completion_tokens": 24,
                        "total_tokens": 84
                    }
                }),
            ]),
        ]);
        let mut runtime =
            build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-deepseek-followup-compat".to_string(),
            TurnInput {
                message: "read Cargo.toml then answer".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("deepseek-followup-compat".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        let events = sink.events.borrow();
        let text_delta = events
            .iter()
            .filter_map(|(name, payload)| (name == "turn:delta").then_some(payload.clone()))
            .find(|payload| payload.text.as_deref() == Some(final_text))
            .expect("text delta event");
        let completed = events
            .iter()
            .find_map(|(name, payload)| (name == "turn:completed").then_some(payload.clone()))
            .expect("completed event");

        assert_eq!(requests.len(), 2);
        let decision_request: Value =
            serde_json::from_str(&requests[0]).expect("decision request should be json");
        let followup_request: Value =
            serde_json::from_str(&requests[1]).expect("followup request should be json");
        assert_eq!(
            decision_request.get("stream").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            followup_request.get("stream").and_then(Value::as_bool),
            Some(true)
        );
        assert!(followup_request.get("stream_options").is_some());
        let replayed_assistant_message = followup_request
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages.iter().find(|message| {
                    message.get("role").and_then(Value::as_str) == Some("assistant")
                        && message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(|calls| !calls.is_empty())
                            .unwrap_or(false)
                })
            })
            .expect("follow-up request should replay assistant tool call message");
        assert_eq!(
            replayed_assistant_message
                .get("reasoning_content")
                .and_then(Value::as_str),
            Some("need workspace listing before answering")
        );
        assert_eq!(text_delta.text.as_deref(), Some(final_text));
        assert_eq!(completed.phase.as_deref(), Some("completed"));
        assert_eq!(
            completed.provider_source.as_deref(),
            Some("provider_followup_stream")
        );
        assert_eq!(completed.fallback_reason, None);
        let provider_calls = completed
            .provider_call_records
            .as_ref()
            .expect("provider call records");
        assert_eq!(provider_calls.len(), 2);
        assert!(provider_calls
            .iter()
            .all(|record| record.first_token_latency_ms.is_some()));
    }

    #[test]
    fn deepseek_multi_hop_followup_preserves_structured_reasoning_content() {
        let final_text = "deepseek structured follow-up completed";
        let first_reasoning = json!([
            { "type": "reasoning", "text": "need workspace listing before answering" }
        ]);
        let second_reasoning = json!([
            { "type": "reasoning", "text": "need Cargo.toml content before answering" }
        ]);
        let final_reasoning = json!([
            { "type": "reasoning", "text": "tool output is sufficient" }
        ]);
        let server = MockHttpServer::start(vec![
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": first_reasoning.clone()
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "call a tool first",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_list_files",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_list_files",
                                            "arguments": "{\"path\":\".\",\"limit\":40}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": second_reasoning.clone()
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": "read Cargo.toml next",
                                "tool_calls": [
                                    {
                                        "index": 0,
                                        "id": "call_workspace_read_file",
                                        "type": "function",
                                        "function": {
                                            "name": "workspace_read_file",
                                            "arguments": "{\"path\":\"Cargo.toml\"}"
                                        }
                                    }
                                ]
                            }
                        }
                    ]
                }),
            ]),
            sse_response(&[
                json!({
                    "choices": [
                        {
                            "delta": {
                                "reasoning_content": final_reasoning.clone()
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [
                        {
                            "delta": {
                                "content": final_text
                            }
                        }
                    ]
                }),
                json!({
                    "choices": [],
                    "usage": {
                        "prompt_tokens": 120,
                        "completion_tokens": 36,
                        "total_tokens": 156
                    }
                }),
            ]),
        ]);
        let mut runtime =
            build_runtime_for_test(deepseek_provider_selection(server.base_url.clone()));
        let sink = RecordingTurnEventSink::new();

        runtime.start_turn_stream(
            &sink,
            "turn-deepseek-structured-followup".to_string(),
            TurnInput {
                message: "inspect workspace then read Cargo.toml".to_string(),
                display_message: None,
                provider_id: None,
                model_id: None,
                reasoning_effort: None,
                session_id: Some("deepseek-structured-followup".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
            },
        );

        let requests = server.finish();
        assert_eq!(requests.len(), 3);
        let first_followup: Value =
            serde_json::from_str(&requests[1]).expect("first followup request should be json");
        let second_followup: Value =
            serde_json::from_str(&requests[2]).expect("second followup request should be json");

        let first_replayed = first_followup
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages.iter().find(|message| {
                    message.get("role").and_then(Value::as_str) == Some("assistant")
                        && message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(|calls| !calls.is_empty())
                            .unwrap_or(false)
                })
            })
            .and_then(|message| message.get("reasoning_content"))
            .cloned()
            .expect("first followup should replay structured reasoning");
        let second_replayed = second_followup
            .get("messages")
            .and_then(Value::as_array)
            .and_then(|messages| {
                messages.iter().rev().find(|message| {
                    message.get("role").and_then(Value::as_str) == Some("assistant")
                        && message
                            .get("tool_calls")
                            .and_then(Value::as_array)
                            .map(|calls| !calls.is_empty())
                            .unwrap_or(false)
                })
            })
            .and_then(|message| message.get("reasoning_content"))
            .cloned()
            .expect("second followup should replay structured reasoning");

        assert_eq!(
            first_replayed,
            json!([{ "type": "reasoning", "text": "need workspace listing before answering" }])
        );
        assert_eq!(
            second_replayed,
            json!([{ "type": "reasoning", "text": "need Cargo.toml content before answering" }])
        );
    }
}
