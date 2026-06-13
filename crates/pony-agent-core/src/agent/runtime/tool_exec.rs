use crate::agent::capability_bridge::{
    CapabilityFailureKind, CapabilityToolExecutionResult, SkillDescriptor,
    SkillFailureLayer, SkillInvocationRequest, SkillToolExecutionResult,
};
use crate::agent::hooks::{CapabilityMediationHookPoint, HookTraceRecord};
use crate::agent::provider::ProviderManager;
use crate::agent::runtime::{AgentRuntime, TurnInput};
use crate::agent::session::TurnHistoryMessage;
use crate::agent::telemetry::CapabilityInvocationRecord;
use crate::agent::tools::{default_permission_facts_for_name, ToolCall};
use crate::agent::turn_flow::runtime_log;

// ---------------------------------------------------------------------------
// Standalone helpers used by the tool execution methods below
// ---------------------------------------------------------------------------

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
) -> CapabilityInvocationRecord {
    CapabilityInvocationRecord {
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
        permission_facts: Some(default_permission_facts_for_name(&tool_call.name)),
        skill_id: None,
        skill_source_id: None,
        composed_capability_refs: None,
        composed_capability_kinds: None,
        failure_layer: Some(error.to_string()),
    }
}

fn build_blocked_skill_invocation_record(
    tool_call: &ToolCall,
    skill: Option<&SkillDescriptor>,
    error: &str,
) -> CapabilityInvocationRecord {
    CapabilityInvocationRecord {
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
        permission_facts: Some(default_permission_facts_for_name(&tool_call.name)),
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
            crate::agent::turn_flow::preview_text(&result.tool_result.output, 120)
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

// ---------------------------------------------------------------------------
// Impl block
// ---------------------------------------------------------------------------

impl AgentRuntime {
    pub(super) fn resolve_provider(&self, input: &TurnInput) -> ProviderManager {
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

    pub(super) fn resolve_tool_call(
        &self,
        user_message: &str,
        history: &[TurnHistoryMessage],
        available_skills: &[SkillDescriptor],
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

    pub(super) fn execute_capability_tool_call(
        &self,
        tool_call: &ToolCall,
    ) -> CapabilityToolExecutionResult {
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

    pub(super) fn execute_registered_tool_call(
        &self,
        tool_call: &ToolCall,
    ) -> (
        crate::agent::tools::ToolResult,
        CapabilityInvocationRecord,
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
                .unwrap_or(CapabilityInvocationRecord {
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
                    permission_facts: Some(default_permission_facts_for_name(&tool_call.name)),
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

    pub(super) fn execute_skill_tool_call(
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
