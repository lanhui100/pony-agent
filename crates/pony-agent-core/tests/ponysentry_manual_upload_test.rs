//! PonySentry 手动上传 Trace 契约测试（Test as Contract）
//!
//! 冻结契约：
//! 1. `ponysentry::build_agent_trace_payloads_from_turns`：从会话 TurnTraceRecord 列表
//!    映射为 AgentTracePayload 列表（空会话 → 空 Vec；字段逐项映射；totals 汇总）；
//! 2. 自动上报移除契约：`turn_flow::emit_event` 不再调用 `maybe_report_turn_trace`
//!    （静态源断言——生产路径由自动上报改为纯手动触发）；
//! 3. `upload_session_trace` 控制面入口为 `Result<usize, String>`（返回已上报 turn 数）。
//!
//! 红相阶段：上述映射函数未实现、自动调用未移除，编译或断言失败。

use pony_agent_core::agent::ponysentry::{
    build_agent_trace_payloads_from_turns, build_session_aggregated_trace_payload,
    build_session_trace_payloads_sharded, AgentTracePayload, EvalStatus,
};
use pony_agent_core::agent::session::TurnTraceRecord;
use pony_agent_core::agent::telemetry::TurnToolActivity;

fn sample_turn() -> TurnTraceRecord {
    TurnTraceRecord {
        turn_id: "turn-manual-001".to_string(),
        session_id: Some("sess-manual-001".to_string()),
        event_id: Some("evt-1".to_string()),
        event_type: Some("turn.completed".to_string()),
        event_version: Some("1".to_string()),
        sequence: Some(1),
        emitted_at_ms: Some(1710000000000),
        title: "run tests".to_string(),
        phase: "completed".to_string(),
        trace_steps: Vec::new(),
        trace_timeline: Vec::new(),
        tool_activities: vec![TurnToolActivity {
            id: "act-1".to_string(),
            name: "bash".to_string(),
            canonical_tool_name: Some("bash".to_string()),
            display_name_zh: None,
            status: "success".to_string(),
            description: "list files".to_string(),
            arguments_text: Some("ls -la".to_string()),
            result_text: Some("total 0".to_string()),
            duration_seconds: Some(0.02),
            parent_activity_id: None,
            artifacts: None,
            error: None,
            capability_invocation: None,
        }],
        provider_call_records: Vec::new(),
        hook_trace_records: Vec::new(),
        provider_requested_name: Some("deepseek-chat".to_string()),
        provider_name: Some("deepseek".to_string()),
        provider_protocol: Some("openai".to_string()),
        provider_model: Some("deepseek-chat".to_string()),
        provider_source: Some("gateway".to_string()),
        provider_mode: Some("stream".to_string()),
        build_context_observation: None,
        build_context_observation_ref: None,
        session_summary: Some("summary".to_string()),
        fallback_reason: None,
        error: None,
        input_tokens: Some(120),
        cache_hit_input_tokens: Some(60),
        reasoning_tokens: Some(10),
        output_tokens: Some(45),
        total_tokens: Some(235),
        first_token_latency_ms: Some(300),
        turn_duration_ms: Some(820),
        input_text: None,
        output_text: None,
        updated_at: 1710000000001,
    }
}

#[test]
fn test_build_agent_trace_payloads_from_empty_turns() {
    let payloads = build_agent_trace_payloads_from_turns("sess-empty", Vec::new());
    assert!(payloads.is_empty(), "空会话必须返回空 Vec，不得构造半成品 payload");
}

#[test]
fn test_build_agent_trace_payloads_from_single_turn_mapping() {
    let traces = vec![sample_turn()];
    let payloads = build_agent_trace_payloads_from_turns("sess-manual-001", traces);

    assert_eq!(payloads.len(), 1, "每个 turn 映射一个 payload");
    let payload: &AgentTracePayload = &payloads[0];
    assert_eq!(payload.session_id, "sess-manual-001");
    assert_eq!(payload.eval_status, EvalStatus::Unreviewed);
    assert_eq!(payload.total_input_tokens, Some(120));
    assert_eq!(payload.total_output_tokens, Some(45));
    assert_eq!(payload.total_duration_ms, Some(820));

    let turn = &payload.turns[0];
    assert_eq!(turn.turn_id, "turn-manual-001");
    assert_eq!(turn.model.as_deref(), Some("deepseek-chat"));
    assert_eq!(turn.cache_hit_tokens, Some(60));
    assert_eq!(turn.duration_ms, Some(820));
    assert_eq!(turn.phase.as_deref(), Some("completed"));

    assert_eq!(turn.tool_calls.len(), 1);
    let tool = &turn.tool_calls[0];
    assert_eq!(tool.tool_name, "bash");
    assert_eq!(tool.status, "success");
    assert_eq!(tool.arguments_summary.as_deref(), Some("ls -la"));
    assert_eq!(tool.duration_ms, Some(20)); // 0.02s -> 20ms
}

#[test]
fn test_build_agent_trace_payloads_multi_turn_totals_per_payload() {
    let traces = vec![sample_turn(), sample_turn()];
    let payloads = build_agent_trace_payloads_from_turns("sess-multi", traces);
    // 升级为业界标准 Session 聚合 Trace：整个 Session 生成 1 个 Payload，包含多轮时序 turns 且全局累加
    assert_eq!(payloads.len(), 1, "同一 Session 必须聚合为单个 Trace Payload");
    let p = &payloads[0];
    assert_eq!(p.turns.len(), 2, "Payload 内部承载全部 2 轮 turns");
    assert_eq!(p.total_input_tokens, Some(240), "全局 input tokens 累加");
    assert_eq!(p.total_output_tokens, Some(90), "全局 output tokens 累加");
    assert_eq!(p.total_duration_ms, Some(1640), "全局 duration 累加");
}

#[test]
fn test_auto_report_removed_from_emit_event() {
    // 自动上报移除契约：turn_flow::emit_event 不得再调用 maybe_report_turn_trace
    let source = std::fs::read_to_string(
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/src/agent/turn_flow.rs"
        ),
    )
    .expect("turn_flow.rs 必须存在");
    assert!(
        !source.contains("maybe_report_turn_trace"),
        "turn_flow::emit_event 中的自动上报调用必须移除（改为纯手动上传）"
    );
}

#[test]
fn test_upload_session_trace_contract_shape() {
    // 契约：控制面 upload_session_trace 返回 async fn(&str) -> Result<usize, String>（上报 turn 数量）。
    // 该入口在 tauri 命令层（src-tauri/src/lib.rs upload_session_trace）暴露。
}

#[test]
fn test_upload_trace_to_live_sentry_contract() {
    let rt = tokio::runtime::Runtime::new().expect("failed to start tokio runtime");
    rt.block_on(async {
        // 验证向生产或测试 Ingest 上报 Trace 时的网络契约：带上默认 token 时能成功通过鉴权
        let config = pony_agent_core::agent::ponysentry::PonySentryConfig::default();
        assert!(config.client_token.is_some(), "Default config must provide client token for sentry auth");

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();

        let trace_url = format!("{}/api/v1/traces", config.endpoint.trim_end_matches('/'));
        let trace_payload = serde_json::json!({
            "session_id": "test-contract-session",
            "turn_id": "test-turn-contract",
            "environment": "test",
            "release": "0.1.107"
        });

        let mut req = client.post(&trace_url).header("Content-Type", "application/json");
        if let Some(ref token) = config.client_token {
            req = req.header("X-Client-Token", token);
        }

        let resp = req.json(&trace_payload).send().await;
        if let Ok(response) = resp {
            assert_ne!(
                response.status(),
                reqwest::StatusCode::UNAUTHORIZED,
                "Trace upload must not be unauthorized with default client token"
            );
            assert_ne!(
                response.status(),
                reqwest::StatusCode::NOT_FOUND,
                "Trace route /api/v1/traces must not return 404 Not Found"
            );
            assert_eq!(
                response.status(),
                reqwest::StatusCode::CREATED,
                "Trace upload should return 201 Created on live sentry"
            );
            // 谁创建谁清理：清理测试上报生成的临时 trace 记录
            let created_body: serde_json::Value = response.json().await.unwrap();
            if let Some(trace_id) = created_body.get("id").and_then(|v| v.as_str()) {
                let _ = std::process::Command::new("psql")
                    .arg("postgresql://job_copilot:8siud3siDW9s02k33edsFAGDDFSDFllidsk9987sddj@127.0.0.1:30543/ponysentry")
                    .arg("-c")
                    .arg(format!("DELETE FROM traces WHERE id = '{trace_id}';"))
                    .output();
            }
        }
    });
}

#[test]
fn test_build_session_trace_payloads_sharded_greedy_split() {
    let session_id = "sess-shard-test-001";
    let project = Some("pony-agent");

    // 构造 12 个 turns，每个 turn 包含较大的 input_text 和 output_text（约 800+ 字节）
    let mut turns = Vec::new();
    for i in 1..=12 {
        let mut turn = sample_turn();
        turn.turn_id = format!("turn-shard-{:03}", i);
        turn.sequence = Some(i as u64);
        turn.emitted_at_ms = Some(1710000000000 + i as u64 * 1000);
        turn.input_text = Some(format!("User query {:03}: {}", i, "A".repeat(500)));
        turn.output_text = Some(format!("Agent answer {:03}: {}", i, "B".repeat(500)));
        turns.push(turn);
    }

    // 设置 max_bytes = 5000 字节，迫使 12 个 turn 被贪心切分为多个分片
    let payloads = build_session_trace_payloads_sharded(session_id, project, turns.clone(), 5000);

    // 断言产出分片数 > 1
    assert!(
        payloads.len() > 1,
        "预期分片数 > 1，实际分片数: {}",
        payloads.len()
    );

    // 收集所有分片的 turns
    let mut collected_turns = Vec::new();
    for (idx, payload) in payloads.iter().enumerate() {
        // 断言各分片的 session_id 严格一致
        assert_eq!(
            payload.session_id, session_id,
            "分片 {} 的 session_id 不匹配",
            idx
        );

        if idx == 0 {
            // 首个分片 carries total_tokens/stats_incomplete/wall_clock/project 元数据
            assert_eq!(payload.project.as_deref(), Some("pony-agent"));
            assert_eq!(payload.total_input_tokens, Some(120 * 12));
            assert_eq!(payload.total_output_tokens, Some(45 * 12));
            assert_eq!(payload.stats_incomplete, false);
            assert!(
                payload.wall_clock_ms.is_some(),
                "首分片应包含 wall_clock_ms"
            );
            assert_eq!(payload.wall_clock_ms, Some(11000)); // 12000 - 1000
        } else {
            // 后续分片无冗余全局元数据
            assert!(
                payload.project.is_none(),
                "后续分片 {} 不应冗余携带 project",
                idx
            );
            assert!(
                payload.total_input_tokens.is_none(),
                "后续分片 {} 不应冗余携带 total_input_tokens",
                idx
            );
            assert!(
                payload.total_output_tokens.is_none(),
                "后续分片 {} 不应冗余携带 total_output_tokens",
                idx
            );
            assert!(
                payload.wall_clock_ms.is_none(),
                "后续分片 {} 不应冗余携带 wall_clock_ms",
                idx
            );
            assert_eq!(payload.stats_incomplete, false);
        }

        for turn in &payload.turns {
            collected_turns.push(turn.clone());
        }
    }

    // 所有分片的 turns 总和与输入相同
    assert_eq!(
        collected_turns.len(),
        turns.len(),
        "分片 turns 总数与输入不一致"
    );

    // 且严格按时间戳递增保序
    for (i, turn) in collected_turns.iter().enumerate() {
        assert_eq!(turn.turn_id, format!("turn-shard-{:03}", i + 1));
        assert_eq!(turn.completed_at_ms, Some(1710000000000 + (i as u64 + 1) * 1000));
    }
}

#[test]
fn test_build_session_aggregated_trace_payload_timestamp_sorting() {
    let session_id = "sess-sort-test";
    // 输入若干 turn，其 emitted_at_ms 分别为 [3000, 1000, 2000]，sequence 为相反顺序
    let mut t1 = sample_turn();
    t1.turn_id = "turn-3000".to_string();
    t1.emitted_at_ms = Some(3000);
    t1.sequence = Some(1);

    let mut t2 = sample_turn();
    t2.turn_id = "turn-1000".to_string();
    t2.emitted_at_ms = Some(1000);
    t2.sequence = Some(3);

    let mut t3 = sample_turn();
    t3.turn_id = "turn-2000".to_string();
    t3.emitted_at_ms = Some(2000);
    t3.sequence = Some(2);

    let input_traces = vec![t1, t2, t3];
    let payload = build_session_aggregated_trace_payload(session_id, None, input_traces)
        .expect("payload should be generated");

    // 断言最终 payload.turns 严格按 [1000, 2000, 3000] 递增排序
    assert_eq!(payload.turns.len(), 3);
    assert_eq!(payload.turns[0].completed_at_ms, Some(1000));
    assert_eq!(payload.turns[0].turn_id, "turn-1000");
    assert_eq!(payload.turns[1].completed_at_ms, Some(2000));
    assert_eq!(payload.turns[1].turn_id, "turn-2000");
    assert_eq!(payload.turns[2].completed_at_ms, Some(3000));
    assert_eq!(payload.turns[2].turn_id, "turn-3000");
}

#[test]
fn test_trace_text_zero_trust_redaction() {
    let session_id = "sess-redact-test";
    let mut turn = sample_turn();
    // 构造 input_text 含敏感路径与 token
    turn.input_text = Some("Deploy key at /home/developer/secrets/api_key.pem with Bearer secret-token-1234567890".to_string());
    turn.output_text = Some("Checking /Users/alice/projects for auth".to_string());

    let payload = build_session_aggregated_trace_payload(session_id, None, vec![turn])
        .expect("payload should be generated");

    assert_eq!(payload.turns.len(), 1);
    let redacted_turn = &payload.turns[0];

    let input = redacted_turn.input_text.as_deref().expect("input_text should exist");
    assert!(
        !input.contains("/home/developer"),
        "敏感绝对路径 /home/developer 必须被脱敏，实际值: {}",
        input
    );
    assert!(
        input.contains("[USER_HOME]/secrets/api_key.pem"),
        "主目录部分必须被替换为 [USER_HOME]，实际值: {}",
        input
    );
    assert!(
        !input.contains("secret-token-1234567890"),
        "Bearer Token 必须被脱敏，实际值: {}",
        input
    );
    assert!(
        input.contains("Bearer [REDACTED_SECRET]"),
        "Bearer Token 必须脱敏为 Bearer [REDACTED_SECRET]，实际值: {}",
        input
    );

    let output = redacted_turn.output_text.as_deref().expect("output_text should exist");
    assert!(
        !output.contains("/Users/alice"),
        "敏感主目录 /Users/alice 必须被脱敏，实际值: {}",
        output
    );
    assert!(
        output.contains("[USER_HOME]/projects"),
        "Mac 主目录部分必须被替换为 [USER_HOME]，实际值: {}",
        output
    );
}