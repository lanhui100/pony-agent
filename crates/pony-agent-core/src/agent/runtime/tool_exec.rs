// tool_exec: provider 解析与 tool / capability / resource / skill 执行。
// 由 runtime/mod.rs 的 AgentRuntime 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl AgentRuntime {
    pub(crate) fn resolve_provider(&self, input: &TurnInput) -> ProviderManager {
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

    pub(crate) fn resolve_tool_call(
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

    pub(crate) fn execute_capability_tool_call(
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
        let failure_kind = if let Some(failure_kind) = tool_result_failure_kind(&tool_result) {
            Some(failure_kind)
        } else if is_out_of_scope_tool_result(&tool_result) {
            Some(CapabilityFailureKind::OutOfScope)
        } else if tool_result.status == "ok" {
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

    pub(crate) fn execute_registered_tool_call(
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
        if is_registry_resource_tool_name(&tool_call.name) {
            let mediated_tool_call = ToolCall {
                arguments: mediation.arguments,
                ..tool_call.clone()
            };
            let (tool_result, invocation_record) =
                self.execute_resource_registry_tool_call(&mediated_tool_call);
            let mut hook_trace_records = mediation.trace_records;
            hook_trace_records.push(build_observe_hook_trace_record(
                "capability.resolve.observe",
                TurnHookPoint::CapabilityResolve,
                1,
                format!(
                    "capability mediation resolved registry resource tool `{}`",
                    tool_call.name
                ),
                Some(format!("tool={}", tool_call.name)),
            ));
            return (tool_result, invocation_record, hook_trace_records);
        }
        if is_registry_tool_search_name(&tool_call.name) {
            let mediated_tool_call = ToolCall {
                arguments: mediation.arguments,
                ..tool_call.clone()
            };
            let (tool_result, invocation_record) =
                self.execute_tool_search_registry_tool_call(&mediated_tool_call);
            let mut hook_trace_records = mediation.trace_records;
            hook_trace_records.push(build_observe_hook_trace_record(
                "capability.resolve.observe",
                TurnHookPoint::CapabilityResolve,
                1,
                format!(
                    "capability mediation resolved registry discovery tool `{}`",
                    tool_call.name
                ),
                Some(format!("tool={}", tool_call.name)),
            ));
            return (tool_result, invocation_record, hook_trace_records);
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

    pub(crate) fn execute_resource_registry_tool_call(
        &self,
        tool_call: &ToolCall,
    ) -> (
        crate::agent::tools::ToolResult,
        crate::agent::telemetry::CapabilityInvocationRecord,
    ) {
        let Some(capability_id) = tool_call
            .arguments
            .get("capabilityId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            let tool_result = crate::agent::tools::ToolResult {
                tool_name: tool_call.name.clone(),
                status: "error".to_string(),
                output: json!({
                    "ok": false,
                    "tool": tool_call.name,
                    "error": {
                        "code": "missing_capability_id",
                        "message": "缺少必填参数 `capabilityId`。"
                    }
                })
                .to_string(),
                duration_ms: 0,
            };
            return (
                tool_result,
                build_blocked_capability_invocation_record(
                    tool_call,
                    "registry resource call missing capabilityId",
                ),
            );
        };

        let arguments = tool_call
            .arguments
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let request = crate::agent::capability_bridge::CapabilityInvocationRequest {
            capability_id: capability_id.to_string(),
            arguments: arguments.clone(),
        };

        let result = match self.capability_registry.resolve_invocation(&request) {
            Ok(crate::agent::capability_bridge::CapabilityBridgeAction::Resource(action)) => self
                .capability_registry
                .resource_fetch_success_result(action, arguments),
            Ok(_) => self
                .capability_registry
                .resource_fetch_failure_result(&request, CapabilityFailureKind::MalformedResponse),
            Err(failure_kind) => self
                .capability_registry
                .resource_fetch_failure_result(&request, failure_kind),
        };

        let failure_kind = result.failure_kind.clone();
        let invocation_record = crate::agent::telemetry::CapabilityInvocationRecord {
            tool_name: tool_call.name.clone(),
            capability_id: result
                .capability
                .as_ref()
                .map(|capability| capability.capability_id.clone())
                .or_else(|| Some(result.requested_capability_id.clone())),
            source_id: result
                .capability
                .as_ref()
                .map(|capability| capability.source_id.clone()),
            source_kind: result
                .capability
                .as_ref()
                .map(|capability| capability.source_kind.as_str().to_string()),
            capability_kind: Some("resource".to_string()),
            invocation_mode: Some("read_only_fetch".to_string()),
            failure_kind: failure_kind.as_ref().map(|kind| kind.as_str().to_string()),
            requires_approval: result
                .capability
                .as_ref()
                .map(|capability| capability.requires_approval),
            host_mediated: result
                .capability
                .as_ref()
                .map(|capability| capability.host_mediated),
            permission_scope: result
                .capability
                .as_ref()
                .map(|capability| capability.permission_scope.clone()),
            permission_facts: result.capability.as_ref().map(|capability| {
                crate::agent::tools::ToolPermissionFacts {
                    requires_approval: Some(capability.requires_approval),
                    permission_scope: Some(capability.permission_scope.clone()),
                    host_mediated: Some(capability.host_mediated),
                    permission_profile: Some(if capability.source_kind.as_str() == "mcp" {
                        "mcp".to_string()
                    } else {
                        "builtin".to_string()
                    }),
                    approval_mode: Some("none".to_string()),
                    decision_source: Some("runtime".to_string()),
                }
            }),
            skill_id: None,
            skill_source_id: None,
            composed_capability_refs: None,
            composed_capability_kinds: None,
            failure_layer: None,
        };

        let tool_result = crate::agent::tools::ToolResult {
            tool_name: tool_call.name.clone(),
            status: if failure_kind.is_none() { "ok".to_string() } else { "error".to_string() },
            output: serde_json::to_string_pretty(&json!({
                "ok": failure_kind.is_none(),
                "tool": tool_call.name,
                "requestedCapabilityId": result.requested_capability_id,
                "capability": result.capability.as_ref().map(|capability| {
                    json!({
                        "capabilityId": capability.capability_id,
                        "sourceId": capability.source_id,
                        "kind": capability.kind.as_str(),
                        "label": capability.label,
                        "permissionScope": capability.permission_scope
                    })
                }),
                "arguments": result.arguments,
                "content": result.content,
                "error": failure_kind.as_ref().map(|kind| {
                    json!({
                        "code": kind.as_str(),
                        "message": match kind {
                            CapabilityFailureKind::CapabilityNotFound => "未找到对应的 MCP resource capability。".to_string(),
                            CapabilityFailureKind::SourceUnavailable => "对应的 MCP resource source 当前不可用。".to_string(),
                            CapabilityFailureKind::PermissionDenied => "当前 MCP resource capability 需要额外审批或受管执行。".to_string(),
                            CapabilityFailureKind::OutOfScope => "MCP resource 请求超出了当前允许范围。".to_string(),
                            CapabilityFailureKind::MalformedResponse => "MCP resource capability 返回了不完整或异常的结构。".to_string(),
                            CapabilityFailureKind::InvocationFailed => "MCP resource capability 执行失败。".to_string(),
                        }
                    })
                }),
                "summary": {
                    "text": if failure_kind.is_none() {
                        "已完成 MCP resource 读取入口调用。"
                    } else {
                        "MCP resource 读取入口调用失败。"
                    }
                }
            }))
            .unwrap_or_else(|_| "{}".to_string()),
            duration_ms: 0,
        };

        (tool_result, invocation_record)
    }

    pub(crate) fn execute_tool_search_registry_tool_call(
        &self,
        tool_call: &ToolCall,
    ) -> (
        crate::agent::tools::ToolResult,
        crate::agent::telemetry::CapabilityInvocationRecord,
    ) {
        let query = tool_call
            .arguments
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or("");
        let source_id = tool_call
            .arguments
            .get("sourceId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let limit = tool_call
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 20) as usize)
            .unwrap_or(8);

        let normalized_query = query.to_ascii_lowercase();
        let candidates = self
            .capability_registry
            .list_capabilities(source_id, Some("tool"))
            .into_iter()
            .filter(|capability| {
                if normalized_query.is_empty() {
                    return true;
                }
                let label = capability.label.to_ascii_lowercase();
                let description = capability.description.to_ascii_lowercase();
                let capability_id = capability.capability_id.to_ascii_lowercase();
                label.contains(&normalized_query)
                    || description.contains(&normalized_query)
                    || capability_id.contains(&normalized_query)
            })
            .take(limit)
            .map(|capability| {
                let confidence = if normalized_query.is_empty() {
                    0.5
                } else {
                    let label = capability.label.to_ascii_lowercase();
                    let description = capability.description.to_ascii_lowercase();
                    let capability_id = capability.capability_id.to_ascii_lowercase();
                    if label == normalized_query || capability_id == normalized_query {
                        0.98
                    } else if label.contains(&normalized_query)
                        || capability_id.contains(&normalized_query)
                    {
                        0.85
                    } else if description.contains(&normalized_query) {
                        0.7
                    } else {
                        0.6
                    }
                };
                // Keep flat fields for backward compatibility while moving
                // default-tool discovery toward the structured `source` contract.
                json!({
                    "capabilityId": capability.capability_id,
                    "tool_name": capability.label,
                    "label": capability.label,
                    "description": capability.description,
                    "source": {
                        "sourceId": capability.source_id,
                        "sourceKind": capability.source_kind.as_str(),
                        "permissionScope": capability.permission_scope
                    },
                    "confidence": confidence,
                    "sourceId": capability.source_id,
                    "sourceKind": capability.source_kind.as_str(),
                    "invocationMode": capability.invocation_mode.as_str(),
                    "permissionScope": capability.permission_scope
                })
            })
            .collect::<Vec<_>>();

        let invocation_record = crate::agent::telemetry::CapabilityInvocationRecord {
            tool_name: tool_call.name.clone(),
            capability_id: None,
            source_id: source_id.map(ToString::to_string),
            source_kind: None,
            capability_kind: Some("tool".to_string()),
            invocation_mode: Some("discovery".to_string()),
            failure_kind: None,
            requires_approval: Some(false),
            host_mediated: Some(false),
            permission_scope: Some("capability.discovery".to_string()),
            permission_facts: Some(default_permission_facts_for_name(&tool_call.name)),
            skill_id: None,
            skill_source_id: None,
            composed_capability_refs: None,
            composed_capability_kinds: None,
            failure_layer: None,
        };

        let tool_result = crate::agent::tools::ToolResult {
            tool_name: tool_call.name.clone(),
            status: "ok".to_string(),
            output: serde_json::to_string_pretty(&json!({
                "ok": true,
                "tool": tool_call.name,
                "query": query,
                "sourceId": source_id,
                "candidateCount": candidates.len(),
                "candidates": candidates,
                "summary": {
                    "text": format!("已返回 {} 个工具候选。", candidates.len())
                }
            }))
            .unwrap_or_else(|_| "{}".to_string()),
            duration_ms: 0,
        };

        (tool_result, invocation_record)
    }

    pub(crate) fn execute_skill_tool_call(
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
            let capability_failure = tool_result_failure_kind(&tool_result).or_else(|| {
                if is_out_of_scope_tool_result(&tool_result) {
                    Some(CapabilityFailureKind::OutOfScope)
                } else if tool_result.status == "ok" {
                    None
                } else {
                    Some(CapabilityFailureKind::InvocationFailed)
                }
            });
            if capability_failure.is_some() {
                failure_layer = Some(SkillFailureLayer::UnderlyingCapabilityExecution);
            }
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
