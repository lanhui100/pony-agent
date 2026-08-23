use super::*;

pub(super) fn blocked_tool_result(tool_call: &ToolCall, error: &str) -> crate::agent::tools::ToolResult {
    crate::agent::tools::ToolResult {
        tool_name: tool_call.name.clone(),
        status: "error".to_string(),
        output: serde_json::to_string_pretty(&json!({
            "ok": false,
            "tool": tool_call.name,
            "error": {
                "code": "hook_blocked",
                "message": error,
            },
            "summary": {
                "text": format!("工具 `{}` 被 hook 阻止执行。", tool_call.name)
            }
        }))
        .unwrap_or_else(|_| "{}".to_string()),
        duration_ms: 0,
    }
}


pub(super) fn build_blocked_capability_invocation_record(
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
        permission_facts: Some(default_permission_facts_for_name(&tool_call.name)),
        skill_id: None,
        skill_source_id: None,
        composed_capability_refs: None,
        composed_capability_kinds: None,
        failure_layer: Some(error.to_string()),
    }
}


pub(super) fn build_blocked_skill_invocation_record(
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


pub(super) fn annotate_capability_tool_activities(
    mut activities: Vec<TurnToolActivity>,
    invocation_record: crate::agent::telemetry::CapabilityInvocationRecord,
) -> Vec<TurnToolActivity> {
    if let Some(parent) = activities.first_mut() {
        parent.capability_invocation = Some(invocation_record);
    }
    activities
}


pub(super) fn build_skill_tool_result(
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
        workspace_mode: None,
                session_id: Some("deepseek-followup-compat".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
        workspace_mode: None,
                session_id: Some("deepseek-followup-compat".to_string()),
                node_id: None,
                history: Vec::new(),
                images: Vec::new(),
                workspace_id: None,
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
