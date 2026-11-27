use pony_agent_core::agent::provider::{
    build_anthropic_test_request_payload, extract_anthropic_usage_from_sse_payload,
};
use pony_agent_core::agent::workspace_hotloader::get_global_workspace_hotloader;
use std::fs;

#[test]
fn red_test_workspace_hotloader_session_preservation_in_context() {
    let temp_dir = std::env::temp_dir().join(format!("test_hotloader_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let agents_md = temp_dir.join("AGENTS.md");
    fs::write(&agents_md, "# Rules Baseline v1").expect("write AGENTS.md");

    let hotloader = get_global_workspace_hotloader();
    let session_id = "test-session-cache-preservation";

    // 第一次调用：建立基线
    let (prefix1, notice1) = hotloader.resolve_session_prompt_context(session_id, 1, &temp_dir);
    assert!(prefix1.contains("# Rules Baseline v1"));
    assert!(notice1.is_none());

    // 模拟 mid-turn 用户修改了 AGENTS.md
    fs::write(&agents_md, "# Rules Baseline v2 Modified").expect("write modified AGENTS.md");

    // 第二次调用：静态前缀必须严格相等（确保 KV Cache 稳定命中！），增量以 Notice 出现
    let (prefix2, notice2) = hotloader.resolve_session_prompt_context(session_id, 2, &temp_dir);
    assert_eq!(prefix1, prefix2, "头部静态前缀必须严格一致，禁止篡改头部导致全量 KV Cache 失效");
    assert!(notice2.is_some(), "中途修改必须以 notice 形式通知");
    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn red_test_anthropic_usage_extracts_cache_tokens() {
    let raw_sse = "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":20,\"cache_creation_input_tokens\":100,\"cache_read_input_tokens\":350}}\n\n";
    let usage = extract_anthropic_usage_from_sse_payload(raw_sse);
    assert_eq!(usage.cache_hit_input_tokens, Some(350), "必须正确提取 Anthropic cache_read_input_tokens");
    assert_eq!(usage.cache_hit_source.as_deref(), Some("usage.cache_read_input_tokens"));
}

#[test]
fn red_test_anthropic_system_request_injects_cache_control() {
    let payload = build_anthropic_test_request_payload("You are Pony Agent", "Hello");
    // 检查 system 字段中是否包含 cache_control
    let system_val = &payload["system"];
    let contains_cache_control = match system_val {
        serde_json::Value::Array(arr) => arr.iter().any(|b| b.get("cache_control").is_some()),
        _ => false,
    };
    assert!(contains_cache_control, "Anthropic 系统提示词必须注入 cache_control 标记");
}

#[test]
fn test_prefix_mutation_reasons_real_collection() {
    use pony_agent_core::agent::context::*;
    use pony_agent_core::agent::provider::PrefixMutationReason;

    let session_context = SessionContext {
        conversation_id: "conv-1".to_string(),
        title: "Session Title".to_string(),
        summary: "Prior summary".to_string(),
        recent_history: Vec::new(),
        recent_attachment_assets: Vec::new(),
        turn_count: 101,
        last_referenced_file: None,
        env_info: None,
    };
    let retrieved = RetrievedContextState {
        turn_context: TurnContext {
            user_message: "hello".to_string(),
            images: Vec::new(),
            references_image: false,
            workspace_mode: None,
        },
        session_context,
        run_state: RunState {
            run_id: Some("run-1".to_string()),
            goal: Some("Test Goal".to_string()),
            phase: None,
            active_turn_id: None,
            last_completed_turn_id: None,
            resume_count: None,
            last_decision_summary: None,
            execution_checkpoint_status: None,
            execution_checkpoint_phase: None,
        },
        long_term_memory: LongTermMemory::default(),
        transcript: TranscriptContext::default(),
    };

    let reasons = pony_agent_core::agent::context::collect_prefix_mutation_reasons(&retrieved, &[], None, Some("history truncated"), false);
    assert!(reasons.contains(&PrefixMutationReason::SessionSummaryChanged), "必须捕获 SessionSummaryChanged");
    assert!(reasons.contains(&PrefixMutationReason::RunGoalChanged), "必须捕获 RunGoalChanged");
    assert!(reasons.contains(&PrefixMutationReason::HistoryBoundaryShifted), "必须捕获 HistoryBoundaryShifted");
}

#[test]
fn test_cross_turn_native_prefix_is_byte_identical_with_trailing_system_reminder() {
    use pony_agent_core::agent::compression::{
        compact_native_transcript_with_summary, wrap_summary_to_message,
    };
    use pony_agent_core::agent::config::{
        ProviderModelCapabilities, ResolvedProviderSelection, ThinkingParamPattern,
    };
    use pony_agent_core::agent::context::{DefaultTurnContextBuilder, TurnContextBuilder};
    use pony_agent_core::agent::provider::{ProviderAuthType, ProviderManager, ProviderProtocol};
    use pony_agent_core::agent::session::{EnvironmentInfo, SessionStore, TurnHistoryMessage};
    use serde_json::json;

    let provider = ProviderManager::new(ResolvedProviderSelection {
        requested_name: "deepseek".to_string(),
        provider_name: "deepseek".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url: "https://example.invalid/v1".to_string(),
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "DEEPSEEK_API_KEY".to_string(),
        api_key: Some("test-key".to_string()),
        model: "deepseek-chat".to_string(),
        temperature: 0.2,
        max_output_tokens: 4096,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: true,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    });
    let builder = DefaultTurnContextBuilder;
    let sid = "cache-stability-e2e";
    let store = SessionStore::memory_only();

    // 模拟 Turn 1（已带环境变量）：[System..., User_1, <system-reminder>_1]
    {
        let mut snap = store.snapshot_at_readonly(Some(sid), None, &[]);
        snap.env_info = Some(EnvironmentInfo {
            cwd: "/tmp/ws".to_string(),
            platform: "linux".to_string(),
            shell: Some("bash".to_string()),
            os_version: None,
            current_date: "2026-11-27".to_string(),
            timezone: Some("UTC".to_string()),
            is_git_repo: true,
            git_branch: Some("main".to_string()),
        });
        snap.turn_count = 1;
        snap.last_referenced_file = Some("a.rs".to_string());
        let r1 = builder.retrieve_context_state("第一步读配置", &[], Some("coding"), &snap, None, None);
        let req1 = builder.build_request("coding", &provider, &r1, &[]);
        let u1_pos = req1
            .native_messages
            .iter()
            .position(|m| m["content"] == "第一步读配置")
            .unwrap();
        let prefix1 = &req1.native_messages[..=u1_pos];

        // 模拟 Turn 1 落盘 + 切换瞬态上下文进入 Turn 2
        snap.history = vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "第一步读配置".to_string(),
                ..Default::default()
            },
            TurnHistoryMessage {
                role: "assistant".to_string(),
                content: "配置已读取".to_string(),
                ..Default::default()
            },
        ];
        snap.provider_native_transcript = vec![
            json!({ "role": "user", "content": "第一步读配置" }),
            json!({ "role": "assistant", "content": "配置已读取", "reasoning_content": "ok" }),
        ];
        snap.turn_count = 2;
        snap.last_referenced_file = Some("b.rs".to_string());
        let r2 = builder.retrieve_context_state("第二步改代码", &[], Some("coding"), &snap, None, None);
        let req2 = builder.build_request("coding", &provider, &r2, &[]);

        // Turn 2 中到 User_1 为止的原生前缀必须与 Turn 1 到 User_1 逐消息全等（KV Cache 命中前提）
        assert_eq!(
            &req2.native_messages[..=u1_pos],
            prefix1,
            "Turn 2 的 [System + User_1] 前缀必须与 Turn 1 字节级完全一致"
        );
        // Volatile 必须作为 role=user 的 <system-reminder> 在尾部，对话中后部绝无 role=system
        let last = req2.native_messages.last().unwrap();
        assert_eq!(last["role"], "user");
        assert!(last["content"].as_str().unwrap().starts_with("<system-reminder>"));
        assert!(!req2.native_messages[u1_pos..]
            .iter()
            .any(|m| m["role"] == "system"));

        // 验证压缩注入 <session_summary> 后 native_tool_flow 请求能直接读到摘要
        let wrapped_summary = wrap_summary_to_message("<session_summary>goal: keep ctx</session_summary>");
        snap.provider_native_transcript = compact_native_transcript_with_summary(
            &snap.provider_native_transcript,
            1,
            Some(&wrapped_summary.content),
        );
        let r3 = builder.retrieve_context_state("第三步验收", &[], Some("coding"), &snap, None, None);
        let req3 = builder.build_request("coding", &provider, &r3, &[]);
        assert!(req3.native_messages.iter().any(|m| {
            m["content"]
                .as_str()
                .is_some_and(|c| c.contains("<session_summary>"))
        }));
    }
}
