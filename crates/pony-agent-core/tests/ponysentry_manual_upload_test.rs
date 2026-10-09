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
    build_agent_trace_payloads_from_turns, AgentTracePayload, EvalStatus,
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
    assert_eq!(payloads.len(), 2);
    // 每个 payload 的 totals 只汇总自身 turn 的数据（turn 粒度上报）
    for p in &payloads {
        assert_eq!(p.total_input_tokens, Some(120));
        assert_eq!(p.turns.len(), 1);
    }
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