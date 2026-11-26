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
