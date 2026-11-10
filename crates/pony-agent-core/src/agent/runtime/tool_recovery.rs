use super::*;

pub(super) fn recover_tool_followup_completion<P: crate::agent::provider::ProviderClient>(
    provider: &P,
    planning_request: &ProviderRequest,
    accumulated_messages: &mut Vec<Value>,
    hop_records: &[ToolTurnHopRecord],
    blocked_tool_call: &ToolCall,
    blocked_assistant_message: Option<&Value>,
    recovery_reason: &str,
    context_observation: &BuildContextObservation,
) -> RecoveredToolFollowup {
    let recovery_request = build_tool_followup_recovery_request(planning_request);
    let synthetic_tool_result =
        build_tool_followup_recovery_tool_result(blocked_tool_call, hop_records, recovery_reason);
    let started_at = Instant::now();
    let mut response = provider_followup_with_mode(
        provider,
        &recovery_request,
        &[],
        accumulated_messages,
        blocked_assistant_message,
        blocked_tool_call,
        &synthetic_tool_result,
        crate::agent::turn_flow::LlmRequestMode::ToolRecovery,
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
            // wave-54 C4：恢复路径无协议信息（ProviderResponse 不带 protocol_label）⇒
            // 不臆造口径，按非 anthropic 规则（input 已含缓存命中）。
            None,
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
    accumulated_messages: &mut Vec<Value>,
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
    let recovery_request = build_tool_followup_recovery_request(planning_request);
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

    let mut response = provider_followup_stream_with_mode(
        provider,
        &recovery_request,
        &[],
        accumulated_messages,
        blocked_assistant_message,
        blocked_tool_call,
        &synthetic_tool_result,
        move |delta| {
            // ADR 0055 P0-2：首包时点信号（StreamStart）不承载内容，也**不参与** UI 首字延迟口径——
            // 此处提前返回，UI 的 `first_token_latency` 语义仍是「首个内容增量」；trace span 侧
            // 的 `llm.call.ttft_ms` 由 `turn_flow` 的独立 `on_delta` 采集，不受本处影响。
            if matches!(delta, ProviderStreamChunk::StreamStart) {
                return;
            }
            // 单次时钟采样：保证 call TTFT ≤ turn TTFT 的数学不变式，
            // 避免两次独立 elapsed() 采样点漂移导致的时序反转 flake。
            let latency = if call_first_token_latency_for_emit.get().is_none()
                || first_token_latency_for_emit.get().is_none()
            {
                let now = Instant::now();
                let mut latency: Option<u64> = None;
                if call_first_token_latency_for_emit.get().is_none() {
                    let value = now.duration_since(started_at_for_emit).as_millis() as u64;
                    call_first_token_latency_for_emit.set(Some(value));
                }
                if first_token_latency_for_emit.get().is_none() {
                    let value = now
                        .duration_since(turn_started_at_for_latency)
                        .as_millis() as u64;
                    first_token_latency_for_emit.set(Some(value));
                    latency = Some(value);
                }
                latency
            } else {
                None
            };
            let mut did_emit = false;
            match delta {
                ProviderStreamChunk::StreamStart => {
                    // ADR 0055 P0-2：首包时点信号，无内容（已在闭包入口提前返回，此臂仅为穷尽性）。
                }
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
        crate::agent::turn_flow::LlmRequestMode::ToolRecovery,
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
            // wave-54 C4：恢复路径无协议信息（ProviderResponse 不带 protocol_label）⇒
            // 不臆造口径，按非 anthropic 规则（input 已含缓存命中）。
            None,
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


/// 构建「冗余/超限工具调用恢复」请求。
///
/// 关键不变量：**`native_messages` 必须原样保留** planning_request 的完整 transcript
/// （system 前缀 / 技能 / 记忆 / 历史对话 / 本轮用户输入）。被阻断的 assistant tool_call
/// 与合成 tool result 由调用方通过 `accumulated_messages` 追加在尾部，与正常 follow-up
/// 的拼装路径一致。
///
/// 历史缺陷（trace `38590d52…` 实证）：旧实现把 `native_messages` 整体替换为
/// `[当前 user_message] + 本轮 hops`；而 native 流程下 provider 直接把该字段当 messages
/// （`provider::openai_request_messages`）⇒ 末轮回答丢失 system prompt 与全部历史，
/// 实测 input 18896 → 2311、cache_read=0。
pub(super) fn build_tool_followup_recovery_request(
    planning_request: &ProviderRequest,
) -> ProviderRequest {
    let mut request = planning_request.clone();
    request.observation = Default::default();
    request
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
        tool_calls: Vec::new(),
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
            // wave-54 C3：占位 usage 无来源标注（未知）。
            origin: None,
            cache_creation_input_tokens: None,
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
    protocol: &crate::agent::provider::ProviderProtocol,
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
    final_response: &ProviderResponse,
) -> Option<Vec<Value>> {
    let mut transcript = vec![provider_native_user_message(user_message)];
    for hop in hop_records {
        transcript.push(tool_request_assistant_message(protocol, hop));
        transcript.push(provider_native_tool_result_message_for_protocol(
            protocol,
            &hop.tool_call,
            &hop.tool_result,
        ));
    }
    transcript.push(final_assistant_message(final_response));
    Some(transcript)
}

/// 失败轮的 native transcript 片段：用户原话 + **已闭合**的工具往返 + 中断标记。
///
/// 与成功路径 [`native_transcript_for_tool_turn`] 的唯一差别是**没有最终 response**：
/// 末条 assistant 只能由「中断事实上报」构造。三条硬约束：
///
/// 1. **闭合性由构造保证**：`hop_records` 每条都是「调用 + 结果」成对记录，绝不留下孤儿
///    `tool_calls`——`sanitize_provider_native_transcript` 看到不闭合往返会清空整段
///    transcript，那比不写更糟。
/// 2. **末条 assistant 必须补**：transcript 若以 `tool` 消息结尾，下一请求会出现
///    「tool 结果后直接接 user」的非法序列（Anthropic 直接 400）。
/// 3. **末条 assistant 必须带 reasoning**：`has_legacy_reasoning_gap` 把「`tool` 之后的
///    无 reasoning assistant」判成 legacy 缺口并清空整段 transcript。这里写入的不是伪造的
///    模型思考，而是**中断事实本身**（下一轮模型据此知道上一轮为什么停在工具结果）。
///
/// 无工具往返 => 返回 `None`（本轮还没产生可恢复的过程事实，交给文本层检查点承载）。
pub(super) fn native_transcript_for_failed_turn(
    protocol: &crate::agent::provider::ProviderProtocol,
    user_message: &str,
    hop_records: &[ToolTurnHopRecord],
) -> Option<Vec<Value>> {
    if hop_records.is_empty() {
        return None;
    }
    let mut transcript = vec![provider_native_user_message(user_message)];
    for hop in hop_records {
        transcript.push(tool_request_assistant_message(protocol, hop));
        transcript.push(provider_native_tool_result_message_for_protocol(
            protocol,
            &hop.tool_call,
            &hop.tool_result,
        ));
    }
    const INTERRUPTED_NOTE: &str =
        "（本轮上游调用中断，未产出最终回复；上面的工具往返是已执行的真实步骤。）";
    transcript.push(provider_native_assistant_message_with_reasoning_value(
        INTERRUPTED_NOTE,
        Some(&Value::String(INTERRUPTED_NOTE.to_string())),
    ));
    Some(transcript)
}


pub(super) fn tool_request_assistant_message(
    protocol: &crate::agent::provider::ProviderProtocol,
    hop: &ToolTurnHopRecord,
) -> Value {
    if protocol.is_anthropic() {
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
        protocol,
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

#[cfg(test)]
mod failed_turn_transcript_tests {
    use super::*;
    use serde_json::json;

    fn hop(call_id: &str, reasoning: Option<&str>) -> ToolTurnHopRecord {
        ToolTurnHopRecord {
            assistant_message: None,
            assistant_output_text: String::new(),
            assistant_reasoning_content: reasoning.map(str::to_string),
            assistant_reasoning_content_value: reasoning.map(|value| Value::String(value.to_string())),
            tool_call: ToolCall {
                call_id: Some(call_id.to_string()),
                name: "workspace_read_file".to_string(),
                arguments: json!({"path": "src/lib.rs"}),
                plan: None,
            },
            tool_result: crate::agent::tools::ToolResult {
                tool_name: "workspace_read_file".to_string(),
                status: "ok".to_string(),
                output: "file body".to_string(),
                duration_ms: 1,
            },
        }
    }

    /// P0 门禁（真实事故 pa_01a1c4d0）：失败轮 transcript 必须**闭合**且**以带 reasoning
    /// 的 assistant 收尾**，否则 `sanitize_provider_native_transcript` 会把整段 transcript
    /// 清空——那比不写更糟（历史被抹掉，会话彻底失忆）。
    #[test]
    fn failed_turn_transcript_is_closed_and_ends_with_assistant() {
        let protocols = [
            crate::agent::provider::ProviderProtocol::OpenAiCompletions,
            crate::agent::provider::ProviderProtocol::AnthropicMessages,
        ];
        for protocol in protocols {
            let hops = vec![hop("call-1", Some("thinking-1")), hop("call-2", None)];
            let transcript =
                native_transcript_for_failed_turn(&protocol, "搜索 vercel agent-desktop", &hops)
                    .expect("有工具往返就必须产出 transcript");

            // 首条是用户原话（真实指令不能被摘要/占位覆盖）。
            assert_eq!(transcript[0]["role"], "user");
            assert!(transcript[0]["content"]
                .as_str()
                .unwrap()
                .contains("vercel agent-desktop"));

            // 末条必须是 assistant，且 reasoning 非空（否则整段被 sanitize 清空）。
            let last = transcript.last().unwrap();
            assert_eq!(last["role"], "assistant");
            assert!(
                last.get("tool_calls").is_none(),
                "末条不得带 tool_calls：孤儿调用会让闭合校验失败"
            );
            let reasoning = last.get("reasoning_content").expect("必须带 reasoning_content");
            assert!(
                !serde_json::to_string(reasoning).unwrap().is_empty(),
                "reasoning_content 不得为空壳"
            );

            // 闭合性：每个声明的 tool_call id 都有且只有一条 tool 结果回应。
            // 闭合性对两种协议统一判定：OpenAI 家族用 `tool_calls[].id` + `role:"tool"`
            // 的 `tool_call_id`；Anthropic 家族用 `content[].tool_use.id` + 下一轮 user
            // 消息里的 `content[].tool_result.tool_use_id`。判据只看「声明 id 是否被回应」，
            // 不假设消息角色。
            let mut declared: Vec<String> = Vec::new();
            let mut answered: Vec<String> = Vec::new();
            for message in &transcript {
                if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
                    for call in calls {
                        declared.push(
                            call.get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        );
                    }
                }
                if let Some(id) = message.get("tool_call_id").and_then(Value::as_str) {
                    answered.push(id.to_string());
                }
                if let Some(blocks) = message.get("content").and_then(Value::as_array) {
                    for block in blocks {
                        match block.get("type").and_then(Value::as_str) {
                            Some("tool_use") => declared.push(
                                block
                                    .get("id")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                            ),
                            Some("tool_result") => answered.push(
                                block
                                    .get("tool_use_id")
                                    .and_then(Value::as_str)
                                    .unwrap_or_default()
                                    .to_string(),
                            ),
                            _ => {}
                        }
                    }
                }
            }
            assert_eq!(declared.len(), 2, "两条 hop 必须都声明");
            for id in &declared {
                assert!(
                    answered.contains(id),
                    "工具调用 {id} 必须配一条结果（未闭合会清空整段 transcript）"
                );
            }
        }
    }

    /// 无工具往返 ⇒ 不写 transcript（本轮还没产生可恢复的过程事实）。
    #[test]
    fn failed_turn_without_hops_writes_nothing() {
        let protocol = crate::agent::provider::ProviderProtocol::OpenAiCompletions;
        assert!(native_transcript_for_failed_turn(&protocol, "hi", &[]).is_none());
    }
}
