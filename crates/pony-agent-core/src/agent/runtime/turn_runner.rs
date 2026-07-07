use crate::agent::capability_bridge::CapabilityRegistry;
use crate::agent::context::TurnContextBuilder;
use crate::agent::execution_control::ExecutionControlRegistry;
use crate::agent::hooks::{
    turn_hook_point_for_capability_mediation_hook_point, turn_hook_point_for_planner_hook_point,
    AgentHookExecutor, AgentHookRegistry, CapabilityMediationEnvelope,
    CapabilityMediationHookPoint, HookFailurePolicy, HookStructuredResult, HookTraceRecord,
    PlannerFactsEnvelope, PlannerHookPoint, TurnHookPoint,
};
use crate::agent::planner::TurnPlanner;
use crate::agent::provider::{ProviderDecision, TokenUsage};
use crate::agent::session::SessionStore;
use crate::agent::telemetry::{ProviderCallCacheRecord, TurnToolActivity, TurnTraceStep};
use crate::agent::tools::{ToolCall, ToolExecutor};
use serde_json::Value;
use std::sync::{Arc, RwLock};

// ── Shared context snapshot from AgentRuntime ──
pub struct TurnContext {
    pub sessions: Arc<RwLock<SessionStore>>,
    pub tool_executor: Arc<dyn ToolExecutor + Send>,
    pub hook_registry: Arc<AgentHookRegistry>,
    pub hook_executor: Arc<dyn AgentHookExecutor + Send + Sync>,
    pub capability_registry: Arc<RwLock<CapabilityRegistry>>,
    pub planner: Arc<dyn TurnPlanner + Send + Sync>,
    pub context_builder: Arc<dyn TurnContextBuilder + Send + Sync>,
}

// ── Free-standing hook dispatch ──

pub struct HookDispatchOutcome {
    pub trace_records: Vec<HookTraceRecord>,
    pub fail_turn_error: Option<String>,
}

pub fn dispatch_hook_trace_records(
    hook_registry: &AgentHookRegistry,
    hook_executor: &dyn AgentHookExecutor,
    hook_point: TurnHookPoint,
) -> HookDispatchOutcome {
    let descriptors = hook_registry.list_for_hook_point(&hook_point);
    let mut records = Vec::with_capacity(descriptors.len());
    let mut fail_turn_error = None;

    for (index, descriptor) in descriptors.into_iter().enumerate() {
        match hook_executor.execute(descriptor, hook_point.clone()) {
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

pub struct CapabilityMediationDispatchOutcome {
    pub arguments: Value,
    pub trace_records: Vec<HookTraceRecord>,
    pub blocked_error: Option<String>,
    pub fail_turn_error: Option<String>,
}

pub fn dispatch_capability_mediation_hooks(
    hook_registry: &AgentHookRegistry,
    hook_executor: &dyn AgentHookExecutor,
    hook_point: CapabilityMediationHookPoint,
    envelope: &CapabilityMediationEnvelope,
) -> CapabilityMediationDispatchOutcome {
    let turn_hook_point = turn_hook_point_for_capability_mediation_hook_point(&hook_point);
    let descriptors = hook_registry.list_for_hook_point(&turn_hook_point);
    let mut records = Vec::with_capacity(descriptors.len());
    let mut execution_results = Vec::new();
    let mut fail_turn_error = None;
    let mut blocked_error = None;

    for (index, descriptor) in descriptors.into_iter().enumerate() {
        match hook_executor.execute_capability_mediation(descriptor, hook_point.clone(), envelope) {
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
        super::apply_capability_argument_patches(
            &hook_point,
            &envelope.argument_summary,
            &execution_results,
        )
        .unwrap_or_else(|error| {
            fail_turn_error = Some(error);
            super::normalized_arguments_from_summary(&envelope.argument_summary)
        })
    } else {
        super::normalized_arguments_from_summary(&envelope.argument_summary)
    };

    CapabilityMediationDispatchOutcome {
        arguments,
        trace_records: records,
        blocked_error,
        fail_turn_error,
    }
}

pub struct PlannerDispatchOutcome {
    pub decision: Option<ProviderDecision>,
    pub selected_tool_call: Option<ToolCall>,
    pub trace_records: Vec<HookTraceRecord>,
    pub blocked_error: Option<String>,
    pub fail_turn_error: Option<String>,
}

pub fn dispatch_planner_hooks(
    hook_registry: &AgentHookRegistry,
    hook_executor: &dyn AgentHookExecutor,
    hook_point: PlannerHookPoint,
    envelope: &PlannerFactsEnvelope,
    decision: Option<ProviderDecision>,
    selected_tool_call: Option<ToolCall>,
) -> PlannerDispatchOutcome {
    let turn_hook_point = turn_hook_point_for_planner_hook_point(&hook_point);
    let descriptors = hook_registry.list_for_hook_point(&turn_hook_point);
    let mut records = Vec::with_capacity(descriptors.len());
    let mut execution_results = Vec::new();
    let mut fail_turn_error = None;
    let mut blocked_error = None;

    for (index, descriptor) in descriptors.into_iter().enumerate() {
        match hook_executor.execute_planner(descriptor, hook_point.clone(), envelope) {
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

    let (decision, selected_tool_call) = if fail_turn_error.is_none() && blocked_error.is_none() {
        super::apply_planner_patches(
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

// ── Tool loop outcome collected during execution ──
#[derive(Default)]
pub struct ToolLoopOutcome {
    pub assistant_message: String,
    pub provider_native_transcript: Option<Vec<Value>>,
    pub provider_source: String,
    pub provider_mode: String,
    pub fallback_reason: Option<String>,
    pub token_usage: Option<TokenUsage>,
    pub trace_steps: Vec<TurnTraceStep>,
    pub tool_activities: Vec<TurnToolActivity>,
    pub hook_trace_records: Vec<HookTraceRecord>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    pub planner_hook_trace_records: Vec<HookTraceRecord>,
    pub provider_call_records: Vec<ProviderCallCacheRecord>,
}

// ── Helper utilities ──

pub fn update_execution_checkpoint(
    control: &ExecutionControlRegistry,
    turn_id: &str,
    phase: &str,
    provider_meta: Option<&super::ProviderEventMeta>,
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
        checkpoint.max_hops = super::max_tool_hops_per_turn();
        checkpoint.active_tool_name = active_tool_name.map(str::to_string);
        checkpoint.trace_steps = trace_steps.to_vec();
        checkpoint.tool_activities = tool_activities.to_vec();
        if let Some(meta) = provider_meta {
            checkpoint.provider_requested_name = Some(meta.requested_name.clone());
            checkpoint.provider_name = Some(meta.provider_name.clone());
            checkpoint.provider_protocol = Some(meta.protocol.clone());
            checkpoint.provider_model = Some(meta.model.clone());
        }
        checkpoint.provider_source = provider_source.map(str::to_string);
        checkpoint.provider_mode = provider_mode.map(str::to_string);
        checkpoint.fallback_reason = fallback_reason.map(str::to_string);
        checkpoint.error = error.map(str::to_string);
        if let Some(s) = status {
            checkpoint.status = s.to_string();
        }
    });
}

pub fn should_cancel_turn(control: &ExecutionControlRegistry, turn_id: &str) -> bool {
    control.is_stop_requested(turn_id)
}
