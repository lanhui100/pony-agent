// message_projection: MessageState 投影与 diff 构建（供 runtime view 使用）。
// 由 control_plane/mod.rs 的 HostControlPlane 巨型 impl 拆分而来，行为与结构保持一致。
use super::*;

impl HostControlPlane {
    pub(crate) fn project_message_state(snapshot: &SessionSnapshot) -> MessageStateSnapshot {
        let mut messages = Vec::new();
        let mut trace_index = 0usize;
        let mut turn_index = 0usize;
        let mut current_turn_id: Option<String> = None;

        for history_message in &snapshot.history {
            if history_message.role == "user" {
                turn_index += 1;
                let trace = snapshot.turn_trace_history.get(trace_index);
                let turn_id = history_message
                    .turn_id
                    .clone()
                    .or_else(|| trace.map(|item| item.turn_id.clone()))
                    .unwrap_or_else(|| format!("history-turn-{turn_index}"));
                current_turn_id = Some(turn_id.clone());
                messages.push(MessageStateEntry {
                    message_id: format!("user-{turn_id}"),
                    turn_id,
                    role: "user".to_string(),
                    content: history_message.content.clone(),
                    attachments: history_message.attachments.clone(),
                    status: Some("done".to_string()),
                    reasoning_content: None,
                    model_name: None,
                    token_count: history_message.token_count,
                    tool_name: None,
                    canonical_tool_name: None,
                    display_name_zh: None,
                    detail: None,
                    duration_seconds: None,
                    error_detail: None,
                });
                continue;
            }

            if history_message.role != "assistant" {
                continue;
            }

            let trace = snapshot.turn_trace_history.get(trace_index);
            let turn_id = history_message
                .turn_id
                .clone()
                .or_else(|| current_turn_id.clone())
                .or_else(|| trace.map(|item| item.turn_id.clone()))
                .unwrap_or_else(|| format!("history-turn-{}", turn_index.max(1)));
            let trace_error = trace.and_then(|item| item.error.clone());
            let has_error = history_message.status.as_ref().is_some_and(|status| {
                matches!(status, crate::agent::session::MessageStatus::Error)
            }) || trace
                .as_ref()
                .is_some_and(|item| item.phase == "failed" || item.error.is_some());
            messages.push(MessageStateEntry {
                message_id: format!("assistant-{turn_id}"),
                turn_id: turn_id.clone(),
                role: "assistant".to_string(),
                content: history_message.content.clone(),
                attachments: Vec::new(),
                status: Some(if has_error { "error" } else { "done" }.to_string()),
                reasoning_content: history_message.reasoning_content.clone(),
                model_name: history_message.model_name.clone(),
                token_count: history_message
                    .token_count
                    .or_else(|| trace.and_then(|item| item.output_tokens)),
                tool_name: None,
                canonical_tool_name: None,
                display_name_zh: None,
                detail: None,
                duration_seconds: None,
                error_detail: trace_error,
            });

            if let Some(trace) = trace {
                for tool in trace
                    .tool_activities
                    .iter()
                    .filter(|tool| tool.status != "planned")
                {
                    let detail = Self::message_tool_detail(tool);
                    messages.push(MessageStateEntry {
                        message_id: format!("tool-{turn_id}-{}", tool.id),
                        turn_id: turn_id.clone(),
                        role: "tool".to_string(),
                        content: tool.result_text.clone().unwrap_or_default(),
                        attachments: Vec::new(),
                        status: Some(Self::message_tool_status(&tool.status).to_string()),
                        reasoning_content: None,
                        model_name: None,
                        token_count: None,
                        tool_name: Some(tool.name.clone()),
                        canonical_tool_name: tool.canonical_tool_name.clone(),
                        display_name_zh: tool.display_name_zh.clone(),
                        detail: (!detail.is_empty()).then_some(detail),
                        duration_seconds: tool.duration_seconds,
                        error_detail: tool.error.as_ref().map(|value| value.to_string()),
                    });
                }
            }

            current_turn_id = None;
            trace_index += 1;
        }

        let mut state = MessageStateSnapshot {
            session_id: snapshot.conversation_id.clone(),
            revision: String::new(),
            messages,
        };
        state.revision = Self::message_state_revision(&state).to_string();
        state
    }

    pub(crate) fn message_tool_status(status: &str) -> &str {
        match status {
            "done" => "done",
            "error" => "error",
            _ => "pending",
        }
    }

    pub(crate) fn message_tool_detail(tool: &crate::agent::telemetry::TurnToolActivity) -> String {
        let mut blocks = Vec::new();
        let description = tool.description.trim();
        if !description.is_empty() {
            blocks.push(description.to_string());
        }
        if let Some(arguments) = tool
            .arguments_text
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            blocks.push(format!("参数\n{arguments}"));
        }
        if let Some(result) = tool
            .result_text
            .as_ref()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
        {
            blocks.push(format!("结果\n{result}"));
        }
        blocks.join("\n")
    }

    pub(crate) fn message_state_revision(state: &MessageStateSnapshot) -> u64 {
        let mut hasher = DefaultHasher::new();
        state.session_id.hash(&mut hasher);
        for message in &state.messages {
            message.message_id.hash(&mut hasher);
            message.turn_id.hash(&mut hasher);
            message.role.hash(&mut hasher);
            message.content.hash(&mut hasher);
            message.status.hash(&mut hasher);
            message.reasoning_content.hash(&mut hasher);
            message.model_name.hash(&mut hasher);
            message.token_count.hash(&mut hasher);
            message.tool_name.hash(&mut hasher);
            message.canonical_tool_name.hash(&mut hasher);
            message.display_name_zh.hash(&mut hasher);
            message.detail.hash(&mut hasher);
            message.duration_seconds.map(f64::to_bits).hash(&mut hasher);
            message.error_detail.hash(&mut hasher);
            for attachment in &message.attachments {
                attachment.id.hash(&mut hasher);
                attachment.asset_id.hash(&mut hasher);
                attachment.name.hash(&mut hasher);
                attachment.mime_type.hash(&mut hasher);
                attachment.relative_path.hash(&mut hasher);
                attachment.size_bytes.hash(&mut hasher);
                attachment.created_at_ms.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    pub(crate) fn diff_message_state(
        before: &MessageStateSnapshot,
        after: &MessageStateSnapshot,
    ) -> MessageStateDelta {
        let ops = if before.messages == after.messages {
            Vec::new()
        } else if after
            .messages
            .iter()
            .zip(before.messages.iter())
            .all(|(left, right)| left == right)
            && after.messages.len() < before.messages.len()
        {
            vec![MessageDeltaOp::TruncateAfter {
                message_id: after
                    .messages
                    .last()
                    .map(|message| message.message_id.clone()),
            }]
        } else if before
            .messages
            .iter()
            .zip(after.messages.iter())
            .all(|(left, right)| left == right)
            && before.messages.len() < after.messages.len()
        {
            vec![MessageDeltaOp::Append {
                messages: after.messages[before.messages.len()..].to_vec(),
            }]
        } else {
            vec![MessageDeltaOp::ReplaceAll {
                messages: after.messages.clone(),
            }]
        };

        MessageStateDelta {
            session_id: after.session_id.clone(),
            base_revision: before.revision.clone(),
            target_revision: after.revision.clone(),
            ops,
        }
    }
}
