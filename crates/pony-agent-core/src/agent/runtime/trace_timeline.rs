use super::*;

pub(super) const MAX_TURN_IMAGES: usize = 3;

pub(super) const MAX_TURN_IMAGE_BYTES: u64 = 24 * 1024 * 1024;

pub(super) fn validate_turn_images(images: &[TurnInputImage]) -> Result<(), String> {
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

pub(super) fn should_recall_recent_images(retrieved: &RetrievedContextState) -> bool {
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

pub(super) fn recalled_image_limit(user_message: &str) -> usize {
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

pub(super) fn native_transcript_for_completed_turn(
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

pub(super) fn top_level_tool_activities(
    tool_activities: &[TurnToolActivity],
) -> Vec<&TurnToolActivity> {
    tool_activities
        .iter()
        .filter(|activity| !activity.id.contains("-planned-") && !activity.id.contains("-child-"))
        .collect()
}

pub(super) fn tool_activities_for_parent(
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

pub(super) fn timeline_state_for_phase(phase: &str) -> String {
    match phase {
        "cancelled" => "cancelled".to_string(),
        "failed" => "error".to_string(),
        _ => "completed".to_string(),
    }
}

/// PA-100：从工具活动里提取真实错误文本，替代旧实现把 description 误写入
/// timeline 条目 error 字段的行为（那会让「错误：」栏显示工具描述，掩盖真实原因）。
///
/// 兼容三种现存序列化形状：
/// - ToolError 经 serde 的 `{kind, message, ...}`（telemetry 主路径）
/// - DispatchError into_outcome 的 `{code, message, ...}`（dispatcher 校验失败）
/// - projection 折叠路径的纯字符串 Value
///
/// 取不到结构化错误时返回 `None`——刻意不回退 description：同一 timeline 条目的
/// text 字段已经承载描述文本，error 回退同一字符串只会复现「错误栏显示描述」的假象。
pub(super) fn turn_tool_activity_error_text(activity: &TurnToolActivity) -> Option<String> {
    if activity.status != "error" && activity.status != "aborted" {
        return None;
    }
    let error = activity.error.as_ref()?;
    let text = match error {
        Value::String(value) => value.clone(),
        Value::Object(map) => {
            let kind = map
                .get("kind")
                .or_else(|| map.get("code"))
                .and_then(Value::as_str);
            let message = map.get("message").and_then(Value::as_str);
            match (kind, message) {
                (Some(kind), Some(message)) => format!("{kind}: {message}"),
                (Some(kind), None) => kind.to_string(),
                (None, Some(message)) => message.to_string(),
                (None, None) => error.to_string(),
            }
        }
        other => other.to_string(),
    };

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    /// 「错误」栏展示截断上限：invalid_arguments 类校验消息可能携带完整 JSON-Schema
    /// 路径，整段透传会撑爆 UI 行高；300 字符足够定位问题。
    const MAX_ERROR_TEXT_CHARS: usize = 300;
    if trimmed.chars().count() <= MAX_ERROR_TEXT_CHARS {
        Some(trimmed.to_string())
    } else {
        let head: String = trimmed.chars().take(MAX_ERROR_TEXT_CHARS).collect();
        Some(format!("{head}…"))
    }
}

/// 在 parent activity 及其分组子活动中定位 parent 自身的错误文本。
pub(super) fn timeline_tool_error_text(
    parent: &TurnToolActivity,
    grouped: &[TurnToolActivity],
) -> Option<String> {
    grouped
        .iter()
        .find(|activity| activity.id == parent.id)
        .and_then(turn_tool_activity_error_text)
}

pub(super) fn build_context_uses_retrieval(
    build_context_observation: &BuildContextObservation,
) -> bool {
    build_context_observation.message_count > 2
        || !build_context_observation.prefix_mutation_reasons.is_empty()
        || !build_context_observation
            .semi_stable_context_text
            .trim()
            .is_empty()
}

pub(super) fn build_stream_started_trace_timeline(
    _user_message: &str,
    provider_meta: &ProviderEventMeta,
    build_context_observation: &BuildContextObservation,
) -> Vec<TraceTimelineEntry> {
    let mut sequence = 1_u64;
    let mut timeline = Vec::new();

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
            build_context_observation_ref: None,
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
        // PA-095 #7（实施后审核 P0）：R4b No duplicate storage——timeline 条目
        // 不再内嵌全量 observation（payload 唯一副本在 build_context_observations
        // 表；读取走 trace 级 ref + 读侧水合）。
        build_context_observation: None,
        build_context_observation_ref: None,
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
        build_context_observation_ref: None,
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

// PA-100 审核更正（A-P3-2 / B-P2-3）：本构建器并非测试专用——turn_stream.rs 仍在
// 生产事件中调用它生成进度态 timeline；持久化终态走 build_persisted_trace_timeline。
// 两个 builder 的字段语义（含 error 提取）必须保持一致。
#[cfg_attr(not(test), allow(dead_code))]
#[allow(clippy::too_many_arguments)]
pub(super) fn build_stream_progress_trace_timeline(
    _user_message: &str,
    provider_meta: &ProviderEventMeta,
    provider_source: Option<&str>,
    provider_mode: Option<&str>,
    build_context_observation: &BuildContextObservation,
    tool_activities: &[TurnToolActivity],
    completed_model_hops: &[ModelHopTraceContent],
    model_output_text: Option<&str>,
    model_reasoning_content: Option<&str>,
    first_token_latency_ms: Option<u64>,
    phase: &str,
) -> Vec<TraceTimelineEntry> {
    let top_level_tools = top_level_tool_activities(tool_activities);
    let model_hops = if phase == "calling_tool" {
        top_level_tools.len().max(completed_model_hops.len()).max(1)
    } else {
        (top_level_tools.len() + 1).max(completed_model_hops.len())
    };
    let mut sequence = 1_u64;
    let mut timeline = Vec::new();

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
            build_context_observation_ref: None,
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
        // PA-095 #7（实施后审核 P0）：同上——timeline 条目不内嵌 observation。
        build_context_observation: None,
        build_context_observation_ref: None,
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
        let completed_model_hop = completed_model_hops.get(model_index);
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
            build_context_observation_ref: None,
            tool_activities: Vec::new(),
            text: completed_model_hop.map(|hop| hop.text.clone()).or_else(|| {
                (phase == "calling_model" && is_last_model)
                    .then(|| model_output_text.map(str::to_string))
                    .flatten()
            }),
            reasoning_content: completed_model_hop
                .and_then(|hop| hop.reasoning_content.clone())
                .or_else(|| {
                    (phase == "calling_model" && is_last_model)
                        .then(|| model_reasoning_content.map(str::to_string))
                        .flatten()
                }),
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
            // PA-100：先于 move 提取真实错误文本，供 call_tool / return_result 两个条目共用。
            let parent_error_text =
                if parent_tool.status == "error" || parent_tool.status == "aborted" {
                    timeline_tool_error_text(parent_tool, &grouped_tool_activities)
                } else {
                    None
                };
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
                build_context_observation_ref: None,
                tool_activities: grouped_tool_activities,
                text: Some(parent_tool.description.clone()),
                reasoning_content: None,
                fallback_reason: None,
                // PA-100：error 字段记录真实错误（kind/code + message），不再误写
                // description；aborted 条目同样提取，state 判定维持原语义不变。
                error: parent_error_text.clone(),
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
            });
            sequence += 1;
            // PA-095 #3：return_result 条目（与事件折叠重建同构——tool/result →
            // return_result；running 中间态不产生）。
            if parent_tool.status != "running" {
                timeline.push(TraceTimelineEntry {
                    id: format!("return-{}", sequence),
                    kind: "return_result".to_string(),
                    label: "RETURN RESULT".to_string(),
                    state: if parent_tool.status == "error" {
                        "error".to_string()
                    } else {
                        "completed".to_string()
                    },
                    sequence,
                    provider_requested_name: None,
                    provider_name: None,
                    provider_protocol: None,
                    provider_model: None,
                    provider_source: None,
                    provider_mode: None,
                    build_context_observation: None,
                    build_context_observation_ref: None,
                    tool_activities: Vec::new(),
                    text: parent_tool.result_text.clone(),
                    reasoning_content: None,
                    fallback_reason: None,
                    // PA-100：同 call_tool 条目——error 记录真实错误，不回退 description。
                    error: parent_error_text.clone(),
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
    }

    timeline
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_persisted_trace_timeline(
    _user_message: &str,
    phase: &str,
    provider_meta: Option<&ProviderEventMeta>,
    provider_source: Option<&str>,
    provider_mode: Option<&str>,
    build_context_observation: Option<&BuildContextObservation>,
    tool_activities: &[TurnToolActivity],
    completed_model_hops: &[ModelHopTraceContent],
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
                build_context_observation_ref: None,
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
        // PA-095 #7（实施后审核 P0）：同上——timeline 条目不内嵌 observation。
        build_context_observation: None,
        build_context_observation_ref: None,
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
        let completed_model_hop = completed_model_hops.get(model_index);
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
            build_context_observation_ref: None,
            tool_activities: Vec::new(),
            text: completed_model_hop.map(|hop| hop.text.clone()),
            reasoning_content: completed_model_hop.and_then(|hop| hop.reasoning_content.clone()),
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
            // PA-100：先于 move 提取真实错误文本，供 call_tool / return_result 两个条目共用。
            let parent_error_text =
                if parent_tool.status == "error" || parent_tool.status == "aborted" {
                    timeline_tool_error_text(parent_tool, &grouped_tool_activities)
                } else {
                    None
                };
            let tool_state = if parent_tool.status == "error" {
                "error".to_string()
            } else {
                "completed".to_string()
            };
            timeline.push(TraceTimelineEntry {
                id: format!("tool-{}", sequence),
                kind: "call_tool".to_string(),
                label: format!("CALL TOOL #{} · {}", model_index + 1, parent_tool.name),
                state: tool_state.clone(),
                sequence,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                build_context_observation_ref: None,
                tool_activities: grouped_tool_activities,
                text: Some(parent_tool.description.clone()),
                reasoning_content: None,
                fallback_reason: None,
                // PA-100：error 字段记录真实错误（kind/code + message），不再误写
                // description；aborted 条目同样提取，state 判定维持原语义不变。
                error: parent_error_text.clone(),
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
            });
            sequence += 1;
            // PA-095 #3：return_result 条目（与事件折叠重建同构——tool/result →
            // return_result）。
            timeline.push(TraceTimelineEntry {
                id: format!("return-{}", sequence),
                kind: "return_result".to_string(),
                label: "RETURN RESULT".to_string(),
                state: tool_state,
                sequence,
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                build_context_observation_ref: None,
                tool_activities: Vec::new(),
                text: parent_tool.result_text.clone(),
                reasoning_content: None,
                fallback_reason: None,
                // PA-100：error 字段记录真实错误（kind/code + message），不再误写
                // description；aborted 条目同样提取，state 判定维持原语义不变。
                error: parent_error_text.clone(),
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
            build_context_observation_ref: None,
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

pub(super) fn build_turn_trace_title(message: &str) -> String {
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
