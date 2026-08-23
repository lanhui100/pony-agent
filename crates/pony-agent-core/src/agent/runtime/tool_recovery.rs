use super::*;

pub(super) fn recover_tool_followup_completion<P: crate::agent::provider::ProviderClient>(
    provider: &P,
    planning_request: &ProviderRequest,
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
    blocked_tool_call: &ToolCall,
    blocked_assistant_message: Option<&Value>,
    recovery_reason: &str,
    context_observation: &BuildContextObservation,
) -> RecoveredToolFollowup {
    let recovery_request = build_tool_followup_recovery_request(
        planning_request,
        provider.protocol_label(),
        user_message,
        hop_records,
    );
    let synthetic_tool_result =
        build_tool_followup_recovery_tool_result(blocked_tool_call, hop_records, recovery_reason);
    let started_at = Instant::now();
    let mut response = provider_followup(
        provider,
        &recovery_request,
        &[],
        &mut vec![],
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
        // 兜底：若模型未产出可读文本，注入本地总结（避免"拦截后零产出"）
        if response.output_text.trim().is_empty() {
            response.output_text = build_local_recovery_fallback_text(&synthetic_tool_result, hop_records);
        }
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
pub(super) fn recover_tool_followup_completion_stream<P: crate::agent::provider::ProviderClient>(
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
    let recovery_request = build_tool_followup_recovery_request(
        planning_request,
        provider.protocol_label(),
        user_message,
        hop_records,
    );
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
    let last_emit_for_client = Rc::new(Cell::new(0u64));
    let last_emit_for_client_clone = Rc::clone(&last_emit_for_client);

    let mut response = provider_followup_stream(
        provider,
        &recovery_request,
        &[],
        &mut vec![],
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
            let mut did_emit = false;
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
                    did_emit = true;
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
                        did_emit = true;
                    }
                }
            }
            if did_emit {
                let now = turn_started_at_for_latency.elapsed().as_millis() as u64;
                let prev = last_emit_for_client_clone.get();
                let gap = now.saturating_sub(prev);
                const MIN_CLIENT_GAP_MS: u64 = 16;
                if prev > 0 && gap < MIN_CLIENT_GAP_MS {
                    std::thread::sleep(std::time::Duration::from_millis(MIN_CLIENT_GAP_MS - gap));
                }
                last_emit_for_client_clone.set(now);
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
        let now = turn_started_at_for_latency.elapsed().as_millis() as u64;
        let prev = last_emit_for_client.get();
        let gap = now.saturating_sub(prev);
        const MIN_CLIENT_GAP_MS: u64 = 16;
        if prev > 0 && gap < MIN_CLIENT_GAP_MS {
            std::thread::sleep(std::time::Duration::from_millis(MIN_CLIENT_GAP_MS - gap));
        }
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
        // 兜底：若模型未产出可读文本，注入本地总结（避免"拦截后零产出"）
        if response.output_text.trim().is_empty() {
            response.output_text = build_local_recovery_fallback_text(&synthetic_tool_result, hop_records);
        }
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


pub(super) fn build_tool_followup_recovery_request(
    planning_request: &ProviderRequest,
    protocol_label: &str,
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
) -> ProviderRequest {
    let mut request = planning_request.clone();
    request.native_messages =
        tool_turn_native_transcript_prefix(protocol_label, user_message, hop_records);
    request.observation = Default::default();
    request
}


pub(super) fn tool_turn_native_transcript_prefix(
    protocol_label: &str,
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
) -> Vec<Value> {
    let mut transcript = vec![provider_native_user_message(user_message)];
    for hop in hop_records {
        transcript.push(tool_request_assistant_message(protocol_label, hop));
        transcript.push(provider_native_tool_result_message_for_protocol(
            protocol_label,
            &hop.tool_call,
            &hop.tool_result,
        ));
    }
    transcript
}


pub(super) fn build_tool_followup_recovery_tool_result(
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


/// 兜底文本：recovery 响应丢弃冗余 tool_call 后，若模型没有给出可用文本（output_text 为空），
/// 本地构造一段基于既有 hop 上下文的总结，保证用户始终获得可读产出，避免"拦截后零产出"。
pub(super) fn build_local_recovery_fallback_text(
    synthetic_tool_result: &crate::agent::tools::ToolResult,
    hop_records: &[ToolTurnHopRecord],
) -> String {
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
    let reason = synthetic_tool_result
        .output
        .lines()
        .next()
        .unwrap_or("已停止重复的工具调用")
        .to_string();
    format!(
        "已停止重复探索并进入本地收口。\n原因：{}\n\n最近已拿到的上下文：\n{}\n\n如需更精确答案，请直接基于以上内容作答，或明确指出信息缺口。",
        preview_text(&reason, 240),
        if recent_summaries.is_empty() {
            "- 暂无可复用的工具结果。".to_string()
        } else {
            recent_summaries
        }
    )
}


pub(super) fn build_local_tool_followup_recovery_response(
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
            cache_hit_source: None,
            reasoning_tokens: None,
            output_tokens: None,
            total_tokens: None,
        }),
    }
}


pub(super) fn local_tool_result_summary(tool_result: &crate::agent::tools::ToolResult) -> String {
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


pub(super) fn native_transcript_for_tool_turn(
    protocol_label: &str,
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
    final_response: &ProviderResponse,
) -> Option<Vec<Value>> {
    let mut transcript = vec![provider_native_user_message(user_message)];
    for hop in hop_records {
        transcript.push(tool_request_assistant_message(protocol_label, hop));
        transcript.push(provider_native_tool_result_message_for_protocol(
            protocol_label,
            &hop.tool_call,
            &hop.tool_result,
        ));
    }
    transcript.push(final_assistant_message(final_response));
    Some(transcript)
}


pub(super) fn tool_request_assistant_message(protocol_label: &str, hop: &ToolTurnHopRecord) -> Value {
    if protocol_label == "anthropic" {
        if let Some(message) = hop.assistant_message.as_ref() {
            if message.get("role").and_then(Value::as_str) == Some("assistant")
                && message.get("content").and_then(Value::as_array).is_some()
            {
                return message.clone();
            }
        }
    }

    let reasoning_value = hop.assistant_reasoning_content_value.clone().or_else(|| {
        hop.assistant_reasoning_content
            .as_ref()
            .map(|reasoning| Value::String(reasoning.clone()))
    });
    provider_native_assistant_tool_call_message_for_protocol(
        protocol_label,
        text_if_present(&hop.assistant_output_text),
        reasoning_value.as_ref(),
        &hop.tool_call,
    )
}


pub(super) fn final_assistant_message(response: &ProviderResponse) -> Value {
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


pub(super) fn text_if_present(text: &str) -> Option<&str> {
    if text.trim().is_empty() {
        None
    } else {
        Some(text)
    }
}


pub(super) fn merge_fallback_reason(existing: Option<String>, next: Option<String>) -> Option<String> {
    match (existing, next) {
        (Some(existing), Some(next)) if !next.trim().is_empty() && existing != next => {
            Some(format!("{} | {}", existing, next))
        }
        (Some(existing), _) => Some(existing),
        (None, Some(next)) if !next.trim().is_empty() => Some(next),
        (None, _) => None,
    }
}
