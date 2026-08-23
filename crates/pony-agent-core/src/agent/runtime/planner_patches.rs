use super::*;

pub(super) fn candidate_capability_ids_for_tool_name(
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

    if let Some(execution_primitive) = canonical_tool_name(raw) {
        let primitive_id = format!("builtin:{execution_primitive}");
        if !candidate_ids.contains(&primitive_id) {
            candidate_ids.push(primitive_id);
        }
    }

    for capability in registry.list_capabilities(None, Some("tool")) {
        if capability.label == raw && !candidate_ids.contains(&capability.capability_id) {
            candidate_ids.push(capability.capability_id);
        }
    }

    candidate_ids
}


pub(crate) fn normalized_arguments_from_summary(summary: &str) -> Value {
    serde_json::from_str(summary).unwrap_or_else(|_| Value::Object(Map::new()))
}


pub(crate) fn apply_capability_argument_patches(
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


pub(super) fn apply_arguments_patch(
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


pub(super) fn parse_hook_patch_value(operation: &HookPatchOperation) -> Result<Value, String> {
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


pub(super) fn summarize_provider_decision(decision: &ProviderDecision) -> String {
    match decision.tool_call.as_ref() {
        Some(tool_call) => format!("decision tool `{}`", tool_call.name),
        None => "decision without tool call".to_string(),
    }
}


pub(crate) fn apply_planner_patches(
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


pub(super) fn ensure_planner_decision(decision: &mut Option<ProviderDecision>) -> &mut ProviderDecision {
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


pub(super) fn parse_planner_provider_decision_patch(
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


pub(super) fn parse_planner_tool_call_patch(operation: &HookPatchOperation) -> Result<ToolCall, String> {
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


pub(super) fn parse_planner_string_patch(operation: &HookPatchOperation) -> Result<String, String> {
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


pub(super) fn apply_graph_decision_patches(
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


pub(super) fn graph_decision_kind_label(kind: &GraphDecisionKind) -> &'static str {
    match kind {
        GraphDecisionKind::Continue => "continue",
        GraphDecisionKind::WaitUser => "wait_user",
        GraphDecisionKind::Pause => "pause",
        GraphDecisionKind::Complete => "complete",
        GraphDecisionKind::Fail => "fail",
        GraphDecisionKind::Cancel => "cancel",
    }
}
