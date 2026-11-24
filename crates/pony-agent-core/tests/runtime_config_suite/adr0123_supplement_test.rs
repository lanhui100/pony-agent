//! ADR 0123 补充修正验收测试（契约矩阵 `.dev-team/contract-matrix-adr0123-supplement.md`）。
//!
//! 覆盖三条冻结契约：
//! - A1：`failed_turn_checkpoint_text` 失败轮检查点文本（新增 pub API）；
//! - D1：`is_interrupt_placeholder_text` 中断注记豁免判据（新增 pub API）；
//! - C1：native transcript 截断后仍保持 `assistant.tool_calls` ↔ `tool` 配对闭合。
//!
//! 零假测试：每个用例均含状态/返回值断言，禁止 `unwrap_or`、`#[ignore]` 与吞异常。

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
};
use pony_agent_core::agent::context::{
    ContextStateQuery, ContextStateRetriever, DefaultContextStateRetriever,
    DefaultTurnContextBuilder, TurnContextBuilder,
};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderManager, ProviderProtocol};
use pony_agent_core::agent::session::{
    build_missing_run_control_audit_summary, failed_turn_checkpoint_text,
    interrupted_turn_checkpoint_text, is_interrupt_placeholder_text, SessionSnapshot,
    TurnHistoryMessage,
};
use serde_json::json;
use std::collections::HashSet;

/// 构造带小窗口的工具 provider：让 native 截断路径（含 skeleton 兜底）必然被触发。
fn make_small_window_tool_provider() -> ProviderManager {
    ProviderManager::new(ResolvedProviderSelection {
        requested_name: "test-openai-small-window".to_string(),
        provider_name: "test-openai-small-window".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        model: "deepseek-v4.1-flash".to_string(),
        base_url: "https://api.example.com/v1".to_string(),
        api_key: Some("test-key".to_string()),
        api_key_env_var: "TEST_API_KEY".to_string(),
        auth_type: ProviderAuthType::Bearer,
        temperature: 0.2,
        max_output_tokens: 256,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        thinking_param_pattern: ThinkingParamPattern::None,
        capabilities: ProviderModelCapabilities {
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: true,
            supports_reasoning: false,
            context_window_tokens: Some(16_384),
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        },
    })
}

/// 组装一个「单轮自身即超预算」的 native 工具往返 transcript：
/// 25 组 `assistant(tool_calls)` + 超大 `tool` 结果（远超 16k 窗口的输入预算）。
fn oversized_single_turn_transcript() -> Vec<serde_json::Value> {
    const TOOL_ROUNDTRIPS: usize = 25;
    const TOOL_OUTPUT_CHARS: usize = 9_000;

    let mut transcript = vec![json!({
        "role": "user",
        "content": "请批量探测所有端口并汇总结果",
    })];

    for index in 0..TOOL_ROUNDTRIPS {
        let call_id = format!("call_probe_{index}");
        transcript.push(json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "id": call_id,
                "type": "function",
                "function": {
                    "name": "Run",
                    "arguments": format!("{{\"command\":\"probe --index {index}\"}}")
                }
            }]
        }));
        transcript.push(json!({
            "role": "tool",
            "tool_call_id": format!("call_probe_{index}"),
            // 唯一可识别的大块标记：若骨架路径生效，该文本必须整体消失。
            "content": format!(
                "PROBE_HEAD_{index}\n{}\nPROBE_TAIL_{index}",
                "x".repeat(TOOL_OUTPUT_CHARS)
            )
        }));
    }

    transcript
}

fn session_with_transcript(transcript: Vec<serde_json::Value>) -> SessionSnapshot {
    SessionSnapshot {
        conversation_id: "test-session-adr0123-supplement".to_string(),
        title: "ADR0123 补充验收".to_string(),
        summary: "摘要".to_string(),
        history: vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "请批量探测所有端口并汇总结果".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "已执行部分步骤".to_string(),
                ..Default::default()
            },
        ],
        attachment_assets: Vec::new(),
        provider_native_transcript: transcript,
        turn_trace_history: Vec::new(),
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        history_state_audit_summary: Default::default(),
        run_control_audit_summary: build_missing_run_control_audit_summary(),
        turn_count: 1,
        last_referenced_file: None,
        updated_at_ms: 0,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: Default::default(),
        resolved_node_id: None,
        latest_node_id: None,
        env_info: None,
        workspace_id: None,
    }
}

// ---------------------------------------------------------------------------
// T-A（契约 A1）：失败轮检查点带失败原因
// ---------------------------------------------------------------------------

#[test]
fn t_a_failed_turn_checkpoint_contains_failure_reason() {
    // Arrange
    let prior_text = "已做一半";
    let tool_activities = vec!["Run 探测".to_string()];
    let failure_reason = "工具 `Run` 已连续失败 4 次";

    // Act
    let checkpoint = failed_turn_checkpoint_text(prior_text, &tool_activities, failure_reason)
        .expect("正文与工具步骤均非空时必须产出检查点文本");

    // Assert
    assert!(
        checkpoint.starts_with("[中断轮次检查点"),
        "检查点前缀必须是契约冻结的 `[中断轮次检查点`，实际: {checkpoint:?}"
    );
    assert!(
        checkpoint.contains("失败原因："),
        "失败轮检查点必须追加 `失败原因：` 行，实际: {checkpoint:?}"
    );
    assert!(
        checkpoint.contains("已连续失败 4 次"),
        "失败轮检查点必须逐字带入 failure_reason，实际: {checkpoint:?}"
    );
    assert!(
        checkpoint.contains(prior_text),
        "既有正文不得因新增失败原因而丢失，实际: {checkpoint:?}"
    );
    assert!(
        checkpoint.contains("已执行的工具步骤：") && checkpoint.contains("- Run 探测"),
        "既有工具步骤段落格式不得改变，实际: {checkpoint:?}"
    );
}

#[test]
fn t_a_empty_failure_reason_is_verbatim_equal_to_legacy_checkpoint() {
    // Arrange
    let prior_text = "已做一半";
    let tool_activities = vec!["Run 探测".to_string()];

    // Act
    let with_empty_reason = failed_turn_checkpoint_text(prior_text, &tool_activities, "");
    let legacy = interrupted_turn_checkpoint_text(prior_text, &tool_activities);
    let expected = legacy.expect("既有实现必须为同样的入参产出检查点文本");

    // Assert
    assert_eq!(
        with_empty_reason.as_deref(),
        Some(expected.as_str()),
        "failure_reason 为空时必须与 interrupted_turn_checkpoint_text 逐字一致"
    );
}

#[test]
fn t_a_all_empty_inputs_return_none() {
    // Arrange
    let no_text = "";
    let no_tools: Vec<String> = Vec::new();
    let no_reason = "";

    // Act
    let checkpoint = failed_turn_checkpoint_text(no_text, &no_tools, no_reason);

    // Assert
    assert!(
        checkpoint.is_none(),
        "三参皆空不得凭空产出检查点文本，实际: {checkpoint:?}"
    );
}

// ---------------------------------------------------------------------------
// T-D（契约 D1）：中断注记豁免判据
// ---------------------------------------------------------------------------

#[test]
fn t_d_interrupt_placeholder_text_is_recognized() {
    // Arrange：三类真实中断注记（前两类字面量取自 runtime/tool_recovery.rs）
    let interrupt_texts = [
        "[中断轮次检查点]\n已执行的工具步骤：",
        "[中断轮次检查点]\n已做一半\n已执行的工具步骤：\n- Run 探测",
        "（本轮上游调用中断，未产出最终回复；上面的工具往返是已执行的真实步骤。）",
        "（本轮由用户终止，未产出最终回复；上面的工具往返是已执行的真实步骤。）",
    ];

    for text in interrupt_texts {
        // Act
        let is_placeholder = is_interrupt_placeholder_text(text);

        // Assert
        assert!(
            is_placeholder,
            "中断注记必须被识别为占位文本，实际判为 false: {text:?}"
        );
    }
}

#[test]
fn t_d_normal_reply_text_is_not_placeholder() {
    // Arrange
    let normal_texts = ["正常回复正文", "目录中有 3 个文件，已查看完毕。"];

    for text in normal_texts {
        // Act
        let is_placeholder = is_interrupt_placeholder_text(text);

        // Assert
        assert!(
            !is_placeholder,
            "普通业务正文不得被判为中断占位文本，实际判为 true: {text:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// T-C（契约 C1）：截断后配对闭合
// ---------------------------------------------------------------------------

#[test]
fn t_c_truncated_native_transcript_keeps_tool_roundtrip_paired() {
    // Arrange：单轮自身即超预算的工具往返（触发 skeleton 兜底路径）
    let provider = make_small_window_tool_provider();
    let session = session_with_transcript(oversized_single_turn_transcript());
    let retriever = DefaultContextStateRetriever;
    let builder = DefaultTurnContextBuilder;
    let user_message = "继续下一步：汇总上一轮探测结果";

    let retrieved = retriever.retrieve(ContextStateQuery {
        user_message,
        images: &[],
        workspace_mode: None,
        session: &session,
        run: None,
        checkpoint: None,
    });

    // Act
    let request = builder.build_request("state-machine-v1", &provider, &retrieved, &[]);
    let messages = &request.native_messages;

    // Assert 1：原生工具回放路径必须产出非空 transcript
    assert!(
        !messages.is_empty(),
        "native 工具回放路径必须产出非空 messages"
    );

    // Assert 2：截断确实发生且走了骨架路径 —— 超大工具输出整体被丢弃
    let serialized = serde_json::to_string(messages).expect("native_messages 必须可序列化");
    assert!(
        !serialized.contains("PROBE_HEAD_0"),
        "超大工具结果必须已被截断丢弃；若仍存在说明未进入 skeleton 兜底路径，用例失去验证目标"
    );

    // Assert 3：骨架仍保留带 tool_calls 的 assistant（说明该轮未被整体清空，配对逻辑真实受检）
    let assistant_call_ids: Vec<Vec<String>> = messages
        .iter()
        .filter(|message| message.get("role").and_then(|r| r.as_str()) == Some("assistant"))
        .filter_map(|message| message.get("tool_calls").and_then(|calls| calls.as_array()))
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| call.get("id").and_then(|id| id.as_str()))
                .map(str::to_string)
                .collect::<Vec<String>>()
        })
        .filter(|ids: &Vec<String>| !ids.is_empty())
        .collect();

    assert!(
        !assistant_call_ids.is_empty(),
        "截断后必须仍保留至少一条带 tool_calls 的 assistant（skeleton 兜底契约），实际 messages: {serialized}"
    );
    let expected_call_ids: usize = assistant_call_ids.iter().map(Vec::len).sum();

    // Assert 4：严格配对闭合扫描 —— 无未闭合 tool_calls、无孤儿 tool
    let mut pending: HashSet<String> = HashSet::new();
    let mut orphan_tool_ids: Vec<String> = Vec::new();
    for message in messages {
        match message.get("role").and_then(|r| r.as_str()) {
            Some("assistant") => {
                if let Some(calls) = message.get("tool_calls").and_then(|c| c.as_array()) {
                    for call in calls {
                        if let Some(id) = call.get("id").and_then(|i| i.as_str()) {
                            pending.insert(id.to_string());
                        }
                    }
                }
            }
            Some("tool") => {
                let tool_call_id = message
                    .get("tool_call_id")
                    .and_then(|i| i.as_str())
                    .map(str::to_string);
                match tool_call_id {
                    Some(id) if pending.contains(&id) => {
                        pending.remove(&id);
                    }
                    Some(id) => orphan_tool_ids.push(id),
                    None => orphan_tool_ids.push("<missing tool_call_id>".to_string()),
                }
            }
            _ => {}
        }
    }

    assert!(
        orphan_tool_ids.is_empty(),
        "截断后不得出现无先行 tool_calls 的孤儿 tool 消息，实际孤儿: {orphan_tool_ids:?}，messages: {serialized}"
    );
    assert!(
        pending.is_empty(),
        "截断后每条 assistant.tool_calls 都必须有匹配 tool_call_id 的 tool 结果，未闭合 ID: {pending:?}，messages: {serialized}"
    );

    // Assert 5：配对闭合不是靠「全部删光」骗过断言 —— 受检工具调用确实存在
    let tool_message_count = messages
        .iter()
        .filter(|message| message.get("role").and_then(|r| r.as_str()) == Some("tool"))
        .count();
    assert!(
        tool_message_count > 0,
        "闭合校验必须建立在真实存在 tool 结果之上，实际 tool 消息数: {tool_message_count}"
    );
    assert_eq!(
        tool_message_count, expected_call_ids,
        "每个 tool_call 必须恰好配一条 tool 结果（不多不少），tool_calls 数: {expected_call_ids}，messages: {serialized}"
    );

    // Assert 6：当前轮用户输入仍位于最后一条（截断不得吞掉本轮输入）
    let last = messages.last().expect("messages 非空已在上文断言");
    assert_eq!(
        last.get("role").and_then(|r| r.as_str()),
        Some("user"),
        "截断后最后一条必须是当前轮用户输入"
    );
    assert_eq!(
        last.get("content").and_then(|c| c.as_str()),
        Some(user_message),
        "截断后最后一条内容必须是当前轮用户输入原文"
    );
}
