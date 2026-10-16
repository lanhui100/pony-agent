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
    build_session_trace_payloads_sharded, init as ponysentry_init,
    send_agent_trace_direct, AgentTracePayload, EvalStatus, PonySentryConfig,
};
use pony_agent_core::agent::session::TurnTraceRecord;
use pony_agent_core::agent::telemetry::TurnToolActivity;
use std::io::ErrorKind;
use std::net::TcpListener;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

/// wave-18b：本文件内触碰进程级 env / 全局 ponysentry 客户端的用例统一串行化，
/// 保证 fail-closed 契约断言的确定性（不依赖测试线程调度运气）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

const MANAGED_ENV_KEYS: [&str; 4] = [
    "PONYSENTRY_CLIENT_TOKEN",
    "PONYSENTRY_ENABLED",
    "PONYSENTRY_INGEST_URL",
    "PONYSENTRY_TEST_LIVE",
];

fn lock_env() -> MutexGuard<'static, ()> {
    ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn clear_managed_env() {
    for key in MANAGED_ENV_KEYS {
        std::env::remove_var(key);
    }
}

/// 读取仓库内源码文件（相对 `crates/pony-agent-core` 的路径）。
fn read_repo_source(relative_to_crate_root: &str) -> String {
    let path = format!("{}/{}", env!("CARGO_MANIFEST_DIR"), relative_to_crate_root);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("契约证据源必须可读（{path}）：{err}"))
}

/// 取 `source` 中 `needle` 之后最多 `len` 个字符的片段（用于签名/委派形状断言）。
fn slice_after<'a>(source: &'a str, needle: &str, len: usize) -> Option<&'a str> {
    source
        .find(needle)
        .map(|index| &source[index..(index + len).min(source.len())])
}

/// 本地回环“出口探针”线程：在 `budget` 内等待 TCP 连接，若收到则读空请求并回 `201 Created`
/// （使 `send_trace_direct_http` 走成功分支），最后返回“是否观测到出口连接”。
///
/// 采用非阻塞 accept + 固定轮询间隔 + 硬截止时间，禁止无界 sleep 概率等待。
fn spawn_egress_probe(budget: Duration) -> (u16, std::thread::JoinHandle<bool>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback egress probe");
    listener
        .set_nonblocking(true)
        .expect("egress probe must be non-blocking");
    let port = listener
        .local_addr()
        .expect("probe local_addr")
        .port();

    let handle = std::thread::spawn(move || {
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                    let mut buf = [0u8; 8192];
                    loop {
                        match std::io::Read::read(&mut stream, &mut buf) {
                            Ok(0) => break,
                            Ok(_) => continue,
                            Err(_) => break,
                        }
                    }
                    let _ = std::io::Write::write_all(
                        &mut stream,
                        b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    );
                    let _ = std::io::Write::flush(&mut stream);
                    return true;
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(_) => return false,
            }
        }
        false
    });

    (port, handle)
}

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

/// wave-18b §11 M12（`review-l3-contract-wave18.json` finding#1，R5）：
/// 原函数体仅两行注释、**零断言**恒真空通过。此处补**真断言**：
/// 1. 控制面入口形状契约（`src-tauri/src/lib.rs` 的 `#[tauri::command] upload_session_trace`
///    => `Result<usize, String>` 且委派给 `control_plane.upload_session_trace`）；
/// 2. 控制面实现把直发错误经 `?` 向上传播（fail-closed 不得吞错）；
/// 3. 行为契约：**set-but-empty `PONYSENTRY_ENABLED` 必须视为未启用**（C2-r3 fail-closed），
///    无凭证时 `send_agent_trace_direct` 必须返回 `Err` 且**零网络出口**。
#[test]
fn test_upload_session_trace_contract_shape() {
    let _guard = lock_env();
    clear_managed_env();

    // ---- 1) 控制面入口形状（tauri 命令层） ----
    let lib_rs = read_repo_source("../../src-tauri/src/lib.rs");
    let entry = slice_after(&lib_rs, "async fn upload_session_trace(", 400)
        .expect("src-tauri/src/lib.rs 必须暴露 `async fn upload_session_trace(`");
    assert!(
        entry.contains("-> Result<usize, String>"),
        "控制面入口 `upload_session_trace` 必须返回 `Result<usize, String>`（上报 turn 数），实际签名片段：{entry}"
    );
    assert!(
        entry.contains("control_plane.upload_session_trace(&session_id).await"),
        "控制面入口必须委派给 `HostControlPlane::upload_session_trace`，实际片段：{entry}"
    );

    // ---- 2) 控制面实现：错误必须经 `?` 传播（不得吞掉 fail-closed 错误） ----
    let control_plane_rs =
        read_repo_source("src/agent/control_plane/query_commands.rs");
    assert!(
        control_plane_rs.contains(
            "pub async fn upload_session_trace(&self, session_id: &str) -> Result<usize, String>"
        ),
        "控制面实现签名必须为 `pub async fn upload_session_trace(&self, session_id: &str) -> Result<usize, String>`"
    );
    assert!(
        control_plane_rs.contains("send_agent_trace_direct(payload).await?;"),
        "控制面实现必须用 `?` 传播直发错误（fail-closed 不得静默吞错）"
    );

    // ---- 3) 行为契约：set-but-empty enabled ⇒ 未启用 ⇒ 无凭证时 Err + 零 egress ----
    let (port, probe) = spawn_egress_probe(Duration::from_secs(3));
    std::env::set_var("PONYSENTRY_INGEST_URL", format!("http://127.0.0.1:{port}"));
    // set-but-empty：C2-r3 冻结为“与 unset 同义 ⇒ false”
    std::env::set_var("PONYSENTRY_ENABLED", "");
    std::env::remove_var("PONYSENTRY_CLIENT_TOKEN");

    let config = PonySentryConfig::from_env();
    assert!(
        !config.enabled,
        "C2-r3：set-but-empty PONYSENTRY_ENABLED 必须解析为 false（fail-closed），\
         否则控制面直发路径会在无凭证时仍尝试上报"
    );
    assert!(
        config.client_token.is_none(),
        "未设置 PONYSENTRY_CLIENT_TOKEN 时 client_token 必须为 None"
    );

    ponysentry_init(config);
    let payload = build_session_aggregated_trace_payload(
        "sess-fail-closed-shape",
        None,
        vec![sample_turn()],
    )
    .expect("sample_turn 必须能构造出聚合 payload");

    let rt = tokio::runtime::Runtime::new().expect("failed to start tokio runtime");
    let result = rt.block_on(send_agent_trace_direct(payload));
    let egress_observed = probe.join().unwrap_or(false);

    assert!(
        result.is_err(),
        "未配置凭证（且 set-but-empty enabled 必须视为未启用）时，`send_agent_trace_direct` 必须 \
         fail-closed 返回 Err，实际返回：{result:?}"
    );
    assert!(
        !egress_observed,
        "fail-closed 路径必须零网络出口，但本地回环探针在 127.0.0.1:{port} 观测到了 TCP 连接"
    );

    clear_managed_env();
}

/// wave-18b R6(a)（`review-l3-contract-wave18.json` finding#2 / §11 M13 恒跑真断言段）：
/// **无 env 依赖、恒跑**的真断言 —— 未配置 `PONYSENTRY_CLIENT_TOKEN` 时
/// `send_agent_trace_direct` 必须返回 `Err` 且不得发起任何网络请求。
///
/// 覆盖两种「未启用」输入：
/// 1. 显式 `PONYSENTRY_ENABLED=false`；
/// 2. **set-but-empty `PONYSENTRY_ENABLED=""`**（C2-r3 fail-closed 边界 —— 当前实现
///    落进 `map` 闭包 ⇒ true，故本用例在当前代码上真实失败）。
///
/// 与 M12 的区别：本用例只聚焦“零网络出口 + Err”这一物理事实（本地回环出口探针），
/// 不依赖任何控制面源码形状断言。
#[test]
fn test_upload_trace_direct_sender_is_fail_closed_without_credentials() {
    let _guard = lock_env();
    clear_managed_env();

    let rt = tokio::runtime::Runtime::new().expect("failed to start tokio runtime");

    // ---- 场景 1：显式 PONYSENTRY_ENABLED=false ----
    let (port, probe) = spawn_egress_probe(Duration::from_secs(2));
    std::env::set_var("PONYSENTRY_INGEST_URL", format!("http://127.0.0.1:{port}"));
    std::env::set_var("PONYSENTRY_ENABLED", "false");
    std::env::remove_var("PONYSENTRY_CLIENT_TOKEN");

    ponysentry_init(PonySentryConfig::from_env());
    let payload =
        build_session_aggregated_trace_payload("sess-no-credential", None, vec![sample_turn()])
            .expect("sample_turn 必须能构造出聚合 payload");
    let result = rt.block_on(send_agent_trace_direct(payload));
    let egress_observed = probe.join().unwrap_or(false);

    assert!(
        result.is_err(),
        "未配置 PONYSENTRY_CLIENT_TOKEN 时必须 fail-closed 返回 Err，实际返回：{result:?}"
    );
    assert!(
        !egress_observed,
        "未配置凭证时不得发起网络请求，但本地回环探针在 127.0.0.1:{port} 观测到了 TCP 连接"
    );

    // ---- 场景 2：set-but-empty PONYSENTRY_ENABLED（C2-r3 冻结断言 7） ----
    let (port2, probe2) = spawn_egress_probe(Duration::from_secs(2));
    std::env::set_var("PONYSENTRY_INGEST_URL", format!("http://127.0.0.1:{port2}"));
    std::env::set_var("PONYSENTRY_ENABLED", "");
    std::env::remove_var("PONYSENTRY_CLIENT_TOKEN");

    ponysentry_init(PonySentryConfig::from_env());
    let payload2 =
        build_session_aggregated_trace_payload("sess-set-but-empty", None, vec![sample_turn()])
            .expect("sample_turn 必须能构造出聚合 payload");
    let result2 = rt.block_on(send_agent_trace_direct(payload2));
    let egress_observed2 = probe2.join().unwrap_or(false);

    assert!(
        result2.is_err(),
        "set-but-empty PONYSENTRY_ENABLED 必须与 unset 同义（未启用）⇒ 无凭证时返回 Err，实际返回：{result2:?}"
    );
    assert!(
        !egress_observed2,
        "set-but-empty PONYSENTRY_ENABLED 必须 fail-closed 零 egress，但本地回环探针在 127.0.0.1:{port2} 观测到了 TCP 连接"
    );

    clear_managed_env();
}

/// wave-18 M6/M7/F2 迁移（contract-matrix-wave18.md §7）：
/// - **M6**：默认配置不再提供 client token（硬编码凭证已清除），删除原
///   `assert!(config.client_token.is_some(), ...)` 断言；
/// - **M7**：本用例原依赖默认 token 打**生产端点** `sentry.ponyjob.top` 并断言 201 Created，
///   现改为**显式 env 门控 + 无凭证即跳过**：默认零 egress，绝不打生产；
///   网络失败不再静默吞掉（原 `if let Ok(response)` 会掩盖失败）；
/// - **F2 止血**：清理步骤中的明文 postgres DSN 已移除，改从 `PONYSENTRY_TEST_DSN` 读取，
///   未设即跳过该清理步骤。通用 password 规则属后续 backlog，不在本波给扫描器扩面。
/// - **wave-18b M13 / R6（`review-l3-contract-wave18.json` finding#2）**：原实现用三个 early
///   `return` 静默 skip，却以 `test ... ok` 上报（等价 `#[ignore]` 但报 `ok`）。现改为：
///   **skip 分支自带真断言**（跳过只能是“未显式 opt-in”的结果；若已显式 opt-in 却无可用
///   凭证，属测试配置错误必须失败），并打印显式 `[skip]` 标记；恒跑真断言段见
///   `test_upload_trace_direct_sender_is_fail_closed_without_credentials` 与
///   `test_upload_session_trace_contract_shape`。不使用 `#[ignore]`（task-5 明令）。
#[test]
fn test_upload_trace_to_live_sentry_contract() {
    let _guard = lock_env();
    let config = pony_agent_core::agent::ponysentry::PonySentryConfig::from_env();
    let live_opt_in = std::env::var("PONYSENTRY_TEST_LIVE").ok();

    // ---- M13 恒跑真断言：skip 分支不得零断言通过 ----
    // 跳过只能是「未显式 opt-in」的结果；若已显式 PONYSENTRY_TEST_LIVE=1 却因
    // 无凭证/未启用而无法验证，属测试配置错误，必须失败而非静默报 ok（R6）。
    if !config.enabled || config.client_token.is_none() {
        assert_ne!(
            live_opt_in.as_deref(),
            Some("1"),
            "已显式 PONYSENTRY_TEST_LIVE=1，但 PONYSENTRY_CLIENT_TOKEN 未配置或 enabled=false：\
             live 契约无法验证，属测试配置错误，不得以静默 skip 冒充 ok（R6/M13）"
        );
        eprintln!(
            "[skip] test_upload_trace_to_live_sentry_contract: fail-closed 默认态\
             （enabled={}, token_configured={}）且未显式 PONYSENTRY_TEST_LIVE=1，\
             为避免真实流量打到端点，跳过 live 契约验证。",
            config.enabled,
            config.client_token.is_some()
        );
        return;
    }

    // 已具备凭证与启用态 ⇒ 必须显式 opt-in（禁止默认把真实流量打到端点）
    assert_eq!(
        live_opt_in.as_deref(),
        Some("1"),
        "已配置凭证且 enabled=true 时必须显式 PONYSENTRY_TEST_LIVE=1 才允许 live 契约验证"
    );

    let token = config
        .client_token
        .clone()
        .filter(|token| !token.trim().is_empty())
        .expect("上面的 fail-closed 分支已保证 token 存在且非空白");

    // F2：清理用 DSN 只能来自环境变量（仓库内不得出现明文凭证）
    let cleanup_dsn = std::env::var("PONYSENTRY_TEST_DSN")
        .ok()
        .filter(|dsn| !dsn.trim().is_empty());
    if cleanup_dsn.is_none() {
        eprintln!(
            "[skip-cleanup] test_upload_trace_to_live_sentry_contract: PONYSENTRY_TEST_DSN 未设，\
             跳过上报记录清理步骤（谁创建谁清理的前提是提供隔离库 DSN）。"
        );
    }

    let rt = tokio::runtime::Runtime::new().expect("failed to start tokio runtime");
    rt.block_on(async move {
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

        let resp = client
            .post(&trace_url)
            .header("Content-Type", "application/json")
            .header("X-Client-Token", &token)
            .json(&trace_payload)
            .send()
            .await;

        let response = match resp {
            Ok(response) => response,
            Err(err) => panic!(
                "live sentry trace upload failed (显式 opt-in 下不得静默吞掉网络失败): {err}"
            ),
        };

        assert_ne!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "Trace upload must not be unauthorized with the explicitly provided client token"
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

        // 谁创建谁清理：仅在显式提供隔离库 DSN 时清理测试上报生成的临时 trace 记录
        if let Some(dsn) = cleanup_dsn {
            let created_body: serde_json::Value = response.json().await.unwrap();
            if let Some(trace_id) = created_body.get("id").and_then(|v| v.as_str()) {
                let _ = std::process::Command::new("psql")
                    .arg(dsn)
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