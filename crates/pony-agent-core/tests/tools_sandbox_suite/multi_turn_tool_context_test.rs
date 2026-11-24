use pony_agent_core::agent::capability_bridge::SkillDescriptor;
use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
};
use pony_agent_core::agent::context::{
    ContextStateQuery, ContextStateRetriever, DefaultContextStateRetriever,
    DefaultTurnContextBuilder, TurnContextBuilder,
};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderManager, ProviderProtocol};
use pony_agent_core::agent::session::{SessionSnapshot, TurnHistoryMessage};
use pony_agent_core::agent::tool_pruner::PRUNE_MARKER;
use serde_json::json;

fn make_openai_tool_provider() -> ProviderManager {
    ProviderManager::new(ResolvedProviderSelection {
        requested_name: "test-openai".to_string(),
        provider_name: "test-openai".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        model: "deepseek-v4.1-flash".to_string(),
        base_url: "https://api.example.com/v1".to_string(),
        api_key: Some("test-key".to_string()),
        api_key_env_var: "TEST_API_KEY".to_string(),
        auth_type: ProviderAuthType::Bearer,
        temperature: 0.2,
        max_output_tokens: 4096,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        thinking_param_pattern: ThinkingParamPattern::None,
        capabilities: ProviderModelCapabilities {
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: true,
            supports_reasoning: false, // 模拟非 thinking 或标准工具模型
            context_window_tokens: Some(64000),
            supports_video_input: false,
            supports_audio_input: false,
            supports_image_output: false,
            supports_video_output: false,
            supports_audio_output: false,
        },
    })
}

#[test]
fn test_multi_turn_tool_transcript_retained_and_pruned_in_new_turn() {
    let provider = make_openai_tool_provider();
    let builder = DefaultTurnContextBuilder;
    let retriever = DefaultContextStateRetriever;

    // 构造第 1 轮产生的原生工具交互流（带有超长输出）
    let large_output = format!("file_head_info\n{}\nfile_tail_summary", "x".repeat(10000));
    let native_transcript = vec![
        json!({
            "role": "user",
            "content": "请查看目录中的文件"
        }),
        json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {
                    "id": "call_ls_001",
                    "type": "function",
                    "function": {
                        "name": "Run",
                        "arguments": "{\"command\":\"ls -la\"}"
                    }
                }
            ]
        }),
        json!({
            "role": "tool",
            "tool_call_id": "call_ls_001",
            "content": large_output
        }),
        json!({
            "role": "assistant",
            "content": "目录中有大量文件，已查看完毕。"
        }),
    ];

    let session = SessionSnapshot {
        conversation_id: "test-session-multi-turn".to_string(),
        title: "多轮工具测试".to_string(),
        summary: "会话摘要".to_string(),
        history: vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "请查看目录中的文件".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "目录中有大量文件，已查看完毕。".to_string(),
                ..Default::default()
            },
        ],
        attachment_assets: Vec::new(),
        provider_native_transcript: native_transcript,
        turn_trace_history: Vec::new(),
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        history_state_audit_summary: Default::default(),
        run_control_audit_summary:
            pony_agent_core::agent::session::build_missing_run_control_audit_summary(),
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
    };

    // 第 2 轮用户提问
    let user_message_turn2 = "上一轮命令的具体输出是什么？";
    let retrieved = retriever.retrieve(ContextStateQuery {
        user_message: user_message_turn2,
        images: &[],
        workspace_mode: None,
        session: &session,
        run: None,
        checkpoint: None,
    });

    let skills: Vec<SkillDescriptor> = Vec::new();
    let request = builder.build_request("state-machine-v1", &provider, &retrieved, &skills);

    // 校验：
    // 1. request.native_messages 必须非空（走原生工具重放模式）
    assert!(
        !request.native_messages.is_empty(),
        "必须生成 native_messages 以保留工具上下文"
    );

    // 2. 检查 native_messages 中的角色序列
    let roles: Vec<&str> = request
        .native_messages
        .iter()
        .filter_map(|m| m.get("role").and_then(|r| r.as_str()))
        .collect();

    // 必须包含 tool 角色
    assert!(
        roles.contains(&"tool"),
        "请求消息中必须包含上一轮的 tool 结果，实际 roles: {:?}",
        roles
    );
    assert!(
        roles.contains(&"assistant"),
        "请求消息中必须包含上一轮的 assistant 消息，实际 roles: {:?}",
        roles
    );

    // 3. 检查 tool 消息内容已被修剪（Pruned），包含了修剪标记
    let tool_msg = request
        .native_messages
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
        .expect("必须找到 tool 消息");

    let tool_content = tool_msg.get("content").and_then(|c| c.as_str()).unwrap();
    assert!(
        tool_content.contains(PRUNE_MARKER),
        "超长工具输出必须包含修剪标记 PRUNE_MARKER"
    );
    assert!(
        tool_content.starts_with("file_head_info"),
        "工具输出开头必须保留"
    );
    assert!(
        tool_content.ends_with("file_tail_summary"),
        "工具输出结尾必须保留"
    );
    assert!(
        tool_content.len() < 10000,
        "修剪后的工具输出长度必须显著小于原始长度"
    );

    // 4. 当前轮的用户提问必须在最后一条
    let last_msg = request.native_messages.last().expect("至少有一条消息");
    assert_eq!(last_msg.get("role").and_then(|r| r.as_str()), Some("user"));
    assert_eq!(
        last_msg.get("content").and_then(|c| c.as_str()),
        Some(user_message_turn2)
    );
}

#[test]
fn test_incomplete_tool_roundtrip_is_safely_recovered_not_wiped() {
    let provider = make_openai_tool_provider();
    let builder = DefaultTurnContextBuilder;
    let retriever = DefaultContextStateRetriever;

    // 构造一个在中间被中断的工具调用（有 call_orphan 但没有 tool result 就断了）
    let incomplete_transcript = vec![
        json!({
            "role": "user",
            "content": "执行某个长任务"
        }),
        json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [
                {
                    "id": "call_orphan_001",
                    "type": "function",
                    "function": {
                        "name": "Run",
                        "arguments": "{\"command\":\"long_job\"}"
                    }
                }
            ]
        }),
    ];

    let session = SessionSnapshot {
        conversation_id: "test-session-incomplete".to_string(),
        title: "中断测试".to_string(),
        summary: "摘要".to_string(),
        history: vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "执行某个长任务".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "用户终止".to_string(),
                ..Default::default()
            },
        ],
        attachment_assets: Vec::new(),
        provider_native_transcript: incomplete_transcript,
        turn_trace_history: Vec::new(),
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        history_state_audit_summary: Default::default(),
        run_control_audit_summary:
            pony_agent_core::agent::session::build_missing_run_control_audit_summary(),
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
    };

    let user_message_turn2 = "继续下一步";
    let retrieved = retriever.retrieve(ContextStateQuery {
        user_message: user_message_turn2,
        images: &[],
        workspace_mode: None,
        session: &session,
        run: None,
        checkpoint: None,
    });

    let skills: Vec<SkillDescriptor> = Vec::new();
    let request = builder.build_request("state-machine-v1", &provider, &retrieved, &skills);

    // 验证：
    // 不管是否存在中断，发出的 native_messages 必须完全闭合且合法（每一个 tool_call 都有对应的 tool result）
    let mut pending_calls = std::collections::HashSet::new();
    for msg in &request.native_messages {
        if msg.get("role").and_then(|r| r.as_str()) == Some("assistant") {
            if let Some(calls) = msg.get("tool_calls").and_then(|c| c.as_array()) {
                for c in calls {
                    if let Some(id) = c.get("id").and_then(|i| i.as_str()) {
                        pending_calls.insert(id.to_string());
                    }
                }
            }
        } else if msg.get("role").and_then(|r| r.as_str()) == Some("tool") {
            if let Some(id) = msg.get("tool_call_id").and_then(|i| i.as_str()) {
                pending_calls.remove(id);
            }
        }
    }

    assert!(
        pending_calls.is_empty(),
        "所有 tool_call 必须严格闭合，未闭合的 ID: {:?}",
        pending_calls
    );
}

#[test]
fn test_store_level_e2e_append_snapshot_and_retrieve_multiturn_flow() {
    use pony_agent_core::agent::session::SessionStore;

    let provider = make_openai_tool_provider();
    let builder = DefaultTurnContextBuilder;
    let retriever = DefaultContextStateRetriever;

    let mut store = SessionStore::memory_only();
    let session_id = "e2e-store-session";

    // Turn 1: 真实的工具轮，通过公开的 store.append_turn 写入
    // 包含工具调用与超长工具输出，reasoning_content 为 null（模拟非 reasoning 模型）
    let large_tool_output = format!("header_log\n{}\ntail_summary", "A".repeat(12000));
    let t1_native_transcript = vec![
        json!({
            "role": "user",
            "content": "执行编译命令"
        }),
        json!({
            "role": "assistant",
            "content": "",
            "reasoning_content": serde_json::Value::Null,
            "tool_calls": [{
                "id": "call_compile_1",
                "type": "function",
                "function": {
                    "name": "run_command",
                    "arguments": "{\"CommandLine\": \"cargo check\"}"
                }
            }]
        }),
        json!({
            "role": "tool",
            "tool_call_id": "call_compile_1",
            "content": large_tool_output
        }),
        json!({
            "role": "assistant",
            "content": "编译完成，无报错。",
            "reasoning_content": serde_json::Value::Null
        }),
    ];

    let snap_t1 = store.append_turn(
        Some(session_id),
        "执行编译命令",
        "编译完成，无报错。",
        Some(t1_native_transcript),
        Vec::new(),
    );

    // 关键断言 1：必须证明 P1-1 修复成功，经过真实 append_turn + snapshot_at + sanitize，
    // 非 reasoning 工具模型的 transcript 绝对未被清空！
    assert!(
        !snap_t1.provider_native_transcript.is_empty(),
        "经过真实 store append_turn 后，非 reasoning 工具模型的 transcript 绝不能被清空！"
    );
    assert_eq!(snap_t1.provider_native_transcript.len(), 4);

    // Turn 2: 纯文本轮（追加普通问答，provider_native_transcript 为 None）
    let snap_t2 = store.append_turn(
        Some(session_id),
        "你觉得这个项目架构如何？",
        "架构整体遵循分层设计，模块清晰。",
        None,
        Vec::new(),
    );

    // Turn 3: 模拟第三轮提问并组装模型请求上下文
    let user_msg_t3 = "结合之前的编译结果和架构，帮我写个部署脚本";
    let retrieved = retriever.retrieve(ContextStateQuery {
        user_message: user_msg_t3,
        images: &[],
        workspace_mode: None,
        session: &snap_t2,
        run: None,
        checkpoint: None,
    });

    let request = builder.build_request("state-machine-v1", &provider, &retrieved, &[]);

    // 关键断言 2：走原生工具回放模式
    assert!(
        !request.native_messages.is_empty(),
        "必须生成 native_messages"
    );

    // 关键断言 3：验证第一轮工具调用存在且被修剪
    let tool_msg = request
        .native_messages
        .iter()
        .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
        .expect("必须包含第一轮的 tool 消息");
    let tool_text = tool_msg.get("content").and_then(|c| c.as_str()).unwrap();
    assert!(
        tool_text.contains(PRUNE_MARKER),
        "超长工具输出必须经过确定性修剪"
    );
    assert!(tool_text.starts_with("header_log"));
    assert!(tool_text.ends_with("tail_summary"));

    // 关键断言 4：验证第二轮纯文本问答没有丢失（P1-2 证明！）
    let contents: Vec<String> = request
        .native_messages
        .iter()
        .filter_map(|m| {
            m.get("content")
                .and_then(|c| c.as_str())
                .map(str::to_string)
        })
        .collect();
    assert!(
        contents
            .iter()
            .any(|c| c.contains("你觉得这个项目架构如何？")),
        "第二轮纯文本提问必须完整保留在上下文中！当前 contents: {:?}",
        contents
    );
    assert!(
        contents.iter().any(|c| c.contains("架构整体遵循分层设计")),
        "第二轮纯文本回答必须完整保留在上下文中！当前 contents: {:?}",
        contents
    );

    // 关键断言 5：第三轮当前输入位于末尾
    let last = request.native_messages.last().unwrap();
    assert_eq!(last.get("role").and_then(|r| r.as_str()), Some("user"));
    assert_eq!(
        last.get("content").and_then(|c| c.as_str()),
        Some(user_msg_t3)
    );
}
