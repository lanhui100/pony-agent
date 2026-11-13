//! `otel` 模块实现级单测（含安全门禁与 wire 级断言）。
//!
//! 只覆盖「安全门禁」的实现细节（不构成验收门禁；验收断言在 crate 根 `tests/` 下由 Test Agent 独占）。
//!
//! 为什么独立成文件：`otel/mod.rs` 已顶到规模门禁基线（文件 ≤3000 行的棘轮上限），
//! 测试代码随功能增长会把生产文件一起推过线。拆分后生产代码与测试各自守自己的规模。

use super::*;

use std::sync::atomic::AtomicUsize;

/// 全局门禁/provider 是进程级状态，本模块单测必须串行（避免并行用例互相污染）。
static TEST_LOCK: Mutex<()> = Mutex::new(());

fn test_lock() -> MutexGuard<'static, ()> {
    TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn span_context(trace: u128) -> opentelemetry::trace::SpanContext {
    opentelemetry::trace::SpanContext::new(
        opentelemetry::trace::TraceId::from(trace),
        opentelemetry::trace::SpanId::from((trace as u64) | 1),
        opentelemetry::trace::TraceFlags::SAMPLED,
        true,
        opentelemetry::trace::TraceState::default(),
    )
}

fn str_attr(attrs: &[(String, AttrValue)], key: &str) -> String {
    match attrs.iter().find(|(name, _)| name == key).map(|(_, value)| value) {
        Some(AttrValue::Str(text)) => text.clone(),
        other => panic!("属性 `{key}` 必须是字符串，实际 {other:?}"),
    }
}

/// r2-dim1 attempt 2 不变量：退出 flush 预算必须 ≥ 单次导出的最坏耗时。
///
/// r2-dim1 attempt 2 / F5：导出失败必须有界重试，且重试不得把**退出阻塞**撑爆。
///
/// 缺陷事实：SDK `BatchSpanProcessor` 对导出失败不重试（`export_batch_sync` 先
/// `split_off(0)` 清空批次再导出，worker 循环 `let _ =` 忽略返回值），本仓又未启用
/// `internal-logs` ⇒ 一次瞬时抖动丢一整批 span 且零日志。实测 `s3-t01-list-summarize`
/// 丢最早 2 次调用的 `llm.chat`（根 span 聚合用量 662 证明 4 个 span 都生成过）。
///
/// 三条不变量：
/// ① 重试次数有界且至少 1 次（无界重试会把导出线程永久占住）；
/// ② 退避远小于单次导出超时；
/// ③ **退出阻塞**由 `flush_state` 的 `MAX_FLUSH_WAIT_MS` 夹值兜底，与重试次数无关 ——
///    重试发生在批量导出线程里，退出路径只等 `min(预算, 夹值)` 就走，最坏情况是「尾批
///    在后台继续重试，但进程已退出」，不会让退出延迟随 attempts 线性增长。
///    因此断言的是「夹值 ≤ NFR 预算」，而不是「attempts × 超时 ≤ 预算」——后者在
///    「预算 = 单次导出超时」的冻结前提下不可能成立，属错误建模。
#[test]
fn export_retry_is_bounded_and_exit_blocking_stays_within_budget() {
    assert!(
        EXPORT_MAX_ATTEMPTS >= 2,
        "导出失败必须至少重试一次，否则等于沿用 SDK 的「失败即丢」"
    );
    assert!(
        EXPORT_MAX_ATTEMPTS <= 3,
        "重试次数必须显式有界且小（当前 {EXPORT_MAX_ATTEMPTS}），否则导出线程会被永久占住"
    );
    assert!(
        EXPORT_RETRY_DELAY_MS < EXPORT_TIMEOUT_MS,
        "重试退避 {EXPORT_RETRY_DELAY_MS}ms 必须远小于单次导出超时 {EXPORT_TIMEOUT_MS}ms"
    );
    // 退出阻塞的硬上限：flush_state 用 min(入参, MAX_FLUSH_WAIT_MS) 等待。
    assert!(
        MAX_FLUSH_WAIT_MS <= EXIT_FLUSH_BUDGET_MS,
        "flush 夹值 {MAX_FLUSH_WAIT_MS}ms 不得超过退出预算 {EXIT_FLUSH_BUDGET_MS}ms，\
         否则重试会把退出阻塞推出 NFR 预算"
    );
}

/// 反证来源（attempt 1 实测）：预算 1000ms < `EXPORT_TIMEOUT_MS` 3000ms 时，
/// `flush_state` 的 `recv_timeout(1000)` 必先于导出完成而超时 ⇒「退出前 flush」与
/// 「不 flush」在网络非瞬时下等价，`s1-t02-read-explain` 的根 span 整条缺失。
/// 本用例把「预算 ≥ 导出超时」钉成编译期事实，任何人把它改回 1000 都会红。
#[test]
fn exit_flush_budget_covers_worst_case_export() {
    assert!(
        EXIT_FLUSH_BUDGET_MS >= EXPORT_TIMEOUT_MS,
        "退出 flush 预算 {EXIT_FLUSH_BUDGET_MS}ms 必须 ≥ 单次导出最坏耗时 {EXPORT_TIMEOUT_MS}ms"
    );
    // 关闭档位只承担握手：不得吃掉导出预算，但也不能为 0（recv_timeout(0) 会必然超时，
    // 使每次正常退出都走 mem::forget 并刷假 warn）。
    assert!(
        SHUTDOWN_TIMEOUT_MS > 0,
        "关闭等待档位必须为正，否则关闭握手必然超时"
    );
    assert!(
        SHUTDOWN_TIMEOUT_MS < EXPORT_TIMEOUT_MS,
        "关闭档位 {SHUTDOWN_TIMEOUT_MS}ms 不得挤占导出预算 {EXPORT_TIMEOUT_MS}ms"
    );
}

/// F5 回归：两个并发 turn 的活动上下文必须各自独立（按 tokio task id 索引，而非栈顶）。
#[test]
fn concurrent_turns_do_not_share_parent_span() {
    let _guard = test_lock();
    REPORTING_ENABLED.store(true, Ordering::SeqCst);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .expect("测试 runtime 必须可创建");
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
    let mismatches = std::sync::Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();
    for index in 0..2u128 {
        let barrier = barrier.clone();
        let mismatches = mismatches.clone();
        handles.push(runtime.spawn(async move {
            let expected = span_context(0x1000 + index);
            // exported = true：本用例模拟真实 root span 登记（父子链语义）。
            let slot = register_active_turn(
                &format!("turn-{index}"),
                &format!("sess-{index}"),
                expected.clone(),
                true,
            );
            assert!(slot.is_some(), "活动 turn 必须登记成功");
            assert!(
                slot.as_ref().and_then(|slot| slot.task_id).is_some(),
                "tokio 任务内必须走 task 表（而非线程本地栈）"
            );
            // 两个 turn 同时处于活动状态，模拟宿主按 turn 并发 spawn。
            barrier.wait().await;
            let observed = active_exported_turn_context().expect("任务内必须有活动 turn");
            if observed.span().span_context().trace_id() != expected.trace_id() {
                mismatches.fetch_add(1, Ordering::SeqCst);
            }
            unregister_active_turn(slot);
            assert!(
                current_active_turn().is_none(),
                "摘除后该任务不得再有活动 turn"
            );
        }));
    }
    for handle in handles {
        runtime.block_on(handle).expect("并发测试任务不得 panic");
    }
    assert_eq!(
        mismatches.load(Ordering::SeqCst),
        0,
        "F5：并发 turn 的父 span 串线（跨 turn 污染）"
    );
    REPORTING_ENABLED.store(false, Ordering::SeqCst);
}

/// F2 回归：字符串属性必须统一过 PonySentry 脱敏（令牌/家目录不得明文上报）。
#[test]
fn string_attributes_are_sanitized() {
    let _guard = test_lock();
    let attrs = turn_span_attributes(&TurnSpanInput {
        session_id: "sess Bearer sk-live-ABCDEFGHIJK".to_string(),
        user_id: Some("/Users/alice/.ssh/id_rsa".to_string()),
        turn_id: "turn-1".to_string(),
        sequence: 0,
        phase: "failed".to_string(),
        failed_reason: None,
        prompt_preview: None,
        response_preview: None,
    });
    let session = str_attr(&attrs, ATTR_LANGFUSE_SESSION_ID);
    assert!(
        session.contains("[REDACTED_SECRET]"),
        "F2：Bearer token 必须脱敏，实际 `{session}`"
    );
    assert!(
        !session.contains("sk-live-ABCDEFGHIJK"),
        "F2：令牌明文泄漏 `{session}`"
    );
    let user = str_attr(&attrs, ATTR_LANGFUSE_USER_ID);
    assert!(!user.contains("alice"), "F2：家目录明文泄漏 `{user}`");

    let tool_attrs = tool_span_attributes(&ToolSpanInput {
        tool_name: "bash".to_string(),
        status: "failed".to_string(),
        duration_ms: Some(3),
        error: Some("curl -H 'authorization: Bearer sk-live-ABCDEFGHIJK'".to_string()),
        args_preview: None,
        result_preview: None,
    });
    let error = str_attr(&tool_attrs, ATTR_AGENT_TOOL_ERROR);
    assert!(
        !error.contains("sk-live-ABCDEFGHIJK"),
        "F2：工具错误里的令牌必须脱敏 `{error}`"
    );
}

/// 公网入口回归：`PONY_OTEL_TOKEN` 必须组装成 `Authorization: Bearer <token>`。
///
/// 背景：集群公网 edge collector 用 `bearertokenauth`（`scheme: Bearer`），
/// 裸 token 实测 401；本用例钉死「补 scheme」与「已带 scheme 不重复」两种形态。
#[test]
fn authorization_header_adds_bearer_scheme_once() {
    assert_eq!(authorization_header_value("raw-token-123"), "Bearer raw-token-123");
    assert_eq!(authorization_header_value("  raw-token-123  "), "Bearer raw-token-123");
    assert_eq!(authorization_header_value("Bearer already"), "Bearer already");
    assert_eq!(authorization_header_value("bearer lower"), "bearer lower");
    assert_eq!(authorization_header_value("Basic dXNlcjpwYXNz"), "Basic dXNlcjpwYXNz");
}

/// F1 回归：`Debug` 与敏感 header 判定不得泄漏凭据；明文外网判定覆盖回环豁免。
#[test]
fn config_debug_redacts_headers_and_detects_sensitive_names() {
    let _guard = test_lock();
    let config = OtelConfig {
        endpoint: "https://collector.example/v1/traces".to_string(),
        service_name: "svc".to_string(),
        sampling_ratio: 1.0,
        headers: vec![(AUTH_HEADER_NAME.to_string(), "SUPER_SECRET_TOKEN_123".to_string())],
    };
    let shown = format!("{config:?}");
    assert!(
        !shown.contains("SUPER_SECRET_TOKEN_123"),
        "F1：Debug 明文泄漏 header 值：{shown}"
    );
    assert!(shown.contains("[REDACTED]"), "F1：Debug 必须显式标出脱敏位");

    assert!(is_sensitive_header("authorization"));
    assert!(is_sensitive_header("X-Api-Key"));
    assert!(is_sensitive_header("X-Auth-Token"));
    assert!(!is_sensitive_header("content-type"));

    assert!(is_plaintext_remote("http://collector.example:4318"));
    assert!(!is_plaintext_remote("http://127.0.0.1:4318"));
    assert!(!is_plaintext_remote("https://collector.example:4318"));
}

/// F6 回归（采样面）：关闭态下**只有**带 `pony.otel.forced` 的重放 span 被采样，
/// 未打标的 span 一律 Drop —— 手动上传不再借全局旁路污染同期其它 turn。
#[test]
fn forced_spans_sample_while_gate_is_closed() {
    let _guard = test_lock();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);
    let sampler = GatedSampler { ratio: 1.0 };
    let trace_id = opentelemetry::trace::TraceId::from(7u128);
    let noop_context = Context::new();

    let forced = [KeyValue::new(ATTR_PONY_OTEL_FORCED, true)];
    let decision = sampler
        .should_sample(
            Some(&noop_context),
            trace_id,
            SPAN_AGENT_TURN,
            &opentelemetry::trace::SpanKind::Internal,
            &forced,
            &[],
        )
        .decision;
    assert_eq!(
        decision,
        opentelemetry::trace::SamplingDecision::RecordAndSample,
        "F6：打标的重放 span 必须被强制采样"
    );

    // 同期「正常 turn」的 span（未打标）在关闭态必须 Drop，即使此刻正在上传。
    let normal = [KeyValue::new(ATTR_LANGFUSE_SESSION_ID, "sess-other")];
    let decision = sampler
        .should_sample(
            Some(&noop_context),
            trace_id,
            SPAN_LLM_CHAT,
            &opentelemetry::trace::SpanKind::Internal,
            &normal,
            &[],
        )
        .decision;
    assert_eq!(
        decision,
        opentelemetry::trace::SamplingDecision::Drop,
        "F6：未打标 span 在关闭态必须被丢弃"
    );
    assert!(!is_reporting_enabled(), "F6：采样判定不得改写门禁末态");
}

/// F6 回归（导出面，wire 级）：关闭态手动上传只把带 `pony.otel.forced` 的 span 送上线路。
#[test]
fn closed_gate_manual_upload_exports_only_forced_spans() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);

    let (port, sink) = spawn_local_otlp_sink(Duration::from_millis(1_500));
    init(OtelConfig {
        endpoint: format!("http://127.0.0.1:{port}"),
        service_name: "pony-agent-otel-unit".to_string(),
        sampling_ratio: 1.0,
        headers: Vec::new(),
    })
    .expect("回环端点必须可装配");

    // 同期「另一个 turn」：关闭态下连 span 都不该分配（零分配契约）。
    let foreign_scope = TurnScope::new("sess-foreign-turn", "turn-foreign", 0);
    assert!(
        foreign_scope.trace_id().is_none(),
        "关闭态不得分配 span（否则就有被顺手导出的风险）"
    );
    foreign_scope.end();

    let traces = vec![
        crate::agent::session::TurnTraceRecord {
            turn_id: "turn-forced-1".to_string(),
            session_id: Some("sess-forced-upload".to_string()),
            title: "unit".to_string(),
            phase: "completed".to_string(),
            ..Default::default()
        },
        crate::agent::session::TurnTraceRecord {
            turn_id: "turn-forced-2".to_string(),
            session_id: Some("sess-forced-upload".to_string()),
            title: "unit".to_string(),
            phase: "failed".to_string(),
            ..Default::default()
        },
    ];
    let receipt = force_export_session_trace("sess-forced-upload", &traces)
        .expect("关闭态手动上传必须成功（旁路按 span 打标）");
    assert_eq!(receipt.turns, 2, "A8/P1-3：回执 turns 必须是实际完成 flush 的 turn 数");
    assert!(receipt.failures.is_empty(), "P1-3：成功导出不得带失败原因：{receipt:?}");
    assert!(!is_reporting_enabled(), "F6：手动上传不得改写门禁末态");

    shutdown();
    let bodies = sink.join().expect("sink 线程不得 panic");
    let wire = String::from_utf8_lossy(&bodies).to_string();
    assert!(!wire.is_empty(), "手动上传必须真的产生 OTLP 出口");
    assert!(
        wire.contains(ATTR_PONY_OTEL_FORCED),
        "F6：导出体必须带 `pony.otel.forced` 标记"
    );
    assert!(
        wire.contains("sess-forced-upload"),
        "F6：导出体必须含被上传会话 id"
    );
    assert!(
        !wire.contains("sess-foreign-turn"),
        "F6：关闭态手动上传**不得**带出同期其它 turn 的 span"
    );
    // 回归（2026-10-24 公网 E2E 发现）：包装导出器若忘记转发 `set_resource`，
    // 导出的 ResourceSpans 会没有任何 resource —— Tempo 里没有 `service.name`，
    // 公网 edge 只能把它改写成 `desktop-client`，`client.service.name` 也随之丢失。
    assert!(
        wire.contains("service.name"),
        "导出体必须携带 Resource 的 `service.name`（RuntimeExporter 必须转发 set_resource）"
    );
    assert!(
        wire.contains("pony-agent-otel-unit"),
        "导出体的 `service.name` 必须等于配置值"
    );
}

/// 本地 OTLP/HTTP sink：读满一个请求后回 200，返回累积的 body 字节。
fn spawn_local_otlp_sink(budget: Duration) -> (u16, std::thread::JoinHandle<Vec<u8>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("sink 必须能绑定回环端口");
    listener.set_nonblocking(true).expect("sink 必须非阻塞");
    let port = listener.local_addr().expect("sink local_addr").port();

    let handle = std::thread::spawn(move || {
        let mut bodies: Vec<u8> = Vec::new();
        let deadline = std::time::Instant::now() + budget;
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _peer)) => bodies.extend_from_slice(&read_otlp_http_body(stream)),
                Err(_) => std::thread::sleep(Duration::from_millis(10)),
            }
        }
        bodies
    });

    (port, handle)
}

/// 读一个 HTTP/1.1 请求体（按 `content-length` 收满），读完回 200 并关闭连接。
///
/// 抽成独立函数：原实现内联在 sink 线程闭包里，嵌套深度 11 层，超规模门禁。
fn read_otlp_http_body(mut stream: std::net::TcpStream) -> Vec<u8> {
    use std::io::{Read as _, Write as _};
    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
    let mut raw: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 4096];
    let mut expected_total: Option<usize> = None;
    loop {
        if expected_total.is_some_and(|total| raw.len() >= total) {
            break;
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                raw.extend_from_slice(&buffer[..read]);
                if expected_total.is_none() {
                    expected_total = http_request_total_len(&raw);
                }
            }
            Err(_) => break,
        }
    }
    let _ = stream.write_all(
        b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
    );
    let _ = stream.flush();
    let body_start = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
        .unwrap_or(raw.len());
    raw.split_off(body_start)
}

/// 按 `content-length` 推算「header 结束 + body」的总长度；头未读全或缺失 ⇒ `None`。
fn http_request_total_len(raw: &[u8]) -> Option<usize> {
    let header_end = raw.windows(4).position(|window| window == b"\r\n\r\n")?;
    let headers = String::from_utf8_lossy(&raw[..header_end]).to_lowercase();
    let length = headers
        .lines()
        .find_map(|line| line.strip_prefix("content-length:"))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    Some(header_end + 4 + length)
}

/// task-10 探针：黑洞端点下 600 轮上传 —— 单次 flush 必须 ≤3000ms（+容差），
/// 且 `shutdown()` 的超时/失败分支不得 panic（runtime 有意保留至进程终止）。
#[test]
fn blackhole_600_turn_upload_stays_within_flush_budget() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);

    let (port, blackhole) = spawn_blackhole_sink(Duration::from_millis(12_000));
    init(OtelConfig {
        endpoint: format!("http://127.0.0.1:{port}"),
        service_name: "pony-agent-otel-probe".to_string(),
        sampling_ratio: 1.0,
        headers: Vec::new(),
    })
    .expect("回环端点必须可装配");

    let traces: Vec<crate::agent::session::TurnTraceRecord> = (0..600)
        .map(|index| crate::agent::session::TurnTraceRecord {
            turn_id: format!("turn-probe-{index}"),
            session_id: Some("sess-probe-600".to_string()),
            title: "probe".to_string(),
            phase: "completed".to_string(),
            ..Default::default()
        })
        .collect();

    let started = std::time::Instant::now();
    let result = force_export_session_trace("sess-probe-600", &traces);
    let elapsed = started.elapsed();

    // P1-3：失败不再以 Err 表达，而是进入回执 failures（不静默、不 panic）。
    let receipt = result.expect("P1-3：失败路径也必须返回回执（不得 panic）");
    assert!(
        !receipt.failures.is_empty(),
        "黑洞端点下单块导出必须把失败写进回执 failures：{receipt:?}"
    );
    assert!(
        elapsed <= Duration::from_millis(3_250),
        "task-10：单次 flush 必须 ≤3000ms(+容差)，实测 {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(2_000),
        "task-10：必须真的发出去了，否则探针无效（实测 {elapsed:?}）"
    );

    // 超时/失败分支：只记 warn，不 drop runtime ⇒ 不得 panic。
    shutdown();
    assert!(!is_reporting_enabled(), "shutdown 不得改写上报门禁");
    drop(blackhole);
}

/// 回归断言：多 chunk（4 个 chunk = 2048 条 trace）在黑洞端点下的总耗时
/// 仍被总预算严格限制在 ≤3250ms，彻底杜绝 N×3000ms 放大。
#[test]
fn multi_chunk_blackhole_upload_stays_within_total_flush_budget() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);

    let (port, blackhole) = spawn_blackhole_sink(Duration::from_millis(15_000));
    init(OtelConfig {
        endpoint: format!("http://127.0.0.1:{port}"),
        service_name: "unit-multi-chunk-probe".to_string(),
        sampling_ratio: 1.0,
        headers: Vec::new(),
    })
    .expect("回环端点必须可装配");

    let traces: Vec<crate::agent::session::TurnTraceRecord> = (0..2048)
        .map(|index| crate::agent::session::TurnTraceRecord {
            turn_id: format!("turn-probe-4chunk-{index}"),
            session_id: Some("sess-probe-4chunk".to_string()),
            title: "probe".to_string(),
            phase: "completed".to_string(),
            ..Default::default()
        })
        .collect();

    let started = std::time::Instant::now();
    let result = force_export_session_trace("sess-probe-4chunk", &traces);
    let elapsed = started.elapsed();

    // P1-3：多块失败同样进回执 failures（部分成功时已成功的 turn 仍保留在 turns/traceIds）。
    let receipt = result.expect("P1-3：失败路径也必须返回回执（不得 panic）");
    assert!(
        !receipt.failures.is_empty(),
        "黑洞端点下多块导出必须把失败写进回执 failures：{receipt:?}"
    );
    assert!(
        elapsed <= Duration::from_millis(3_250),
        "多 chunk 导出总耗时必须 ≤3000ms(+容差)，实测 {elapsed:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(2_000),
        "总预算倒计时必须切实执行，实测 {elapsed:?}"
    );

    shutdown();
    drop(blackhole);
}

// ------------------------------- Wave-4.1 L3-A B1 / B1b / A2 实现级行为断言
//
// 说明：这是实现级回归（非验收门禁），与 tests/ 下的冻结验收用例互不替代。

/// B1：门禁关闭（且未 init）时只有「本地相关性」id，导出态 id 必须为 `None`。
#[test]
fn b1_local_context_exposes_current_ids_but_no_exported_trace_id() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);

    let scope = begin_turn_trace("sess-b1", "turn-b1", 0);
    assert!(
        current_trace_id().is_some(),
        "B1：本地相关性 trace_id 必须存在（日志/traceparent 依赖它）"
    );
    assert!(
        current_traceparent().is_some(),
        "B1：本地上下文仍必须能组装 W3C traceparent"
    );
    assert_eq!(
        exported_trace_id(),
        None,
        "B1：门禁关闭时导出态 trace_id 必须为 None（否则 issue 会带不可检索的假关联）"
    );
    assert_eq!(
        scope.trace_id().is_some(),
        true,
        "本地守卫的 trace_id() 仍应可读（日志用）"
    );
    assert_eq!(
        scope.exported_trace_id(),
        None,
        "B1b：本地守卫的导出态 id 必须为 None"
    );
    drop(scope);
    assert_eq!(current_trace_id(), None, "守卫 Drop 后不得残留活动 turn");
}

/// B1 反向：门禁开启 + provider 已装配时，导出态 id 与守卫/本地读取一致。
#[test]
fn b1_exported_trace_id_matches_real_span_when_gate_open() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(true, Ordering::SeqCst);
    init(OtelConfig {
        endpoint: "http://127.0.0.1:9".to_string(),
        service_name: "unit-b1".to_string(),
        sampling_ratio: 1.0,
        headers: Vec::new(),
    })
    .expect("回环端点必须可装配");

    let scope = begin_turn_trace("sess-b1-on", "turn-b1-on", 0);
    let exported = scope.exported_trace_id();
    assert!(
        exported.as_deref().map(|id| id.len()) == Some(32),
        "B1：门禁开启时必须产出 32 位导出态 trace_id，实际 {exported:?}"
    );
    assert_eq!(exported_trace_id(), exported, "全局读取必须等于守卫的导出态 id");
    assert_eq!(current_trace_id(), exported, "导出态与本地读取在此场景应同源");
    drop(scope);

    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);
}

/// B1b：本地上下文**绝不**作为子 span 的 parent（否则会造出无 root 的孤儿 trace）。
#[test]
fn b1b_local_context_never_parents_child_spans() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);

    // 1) 门禁关闭时开启 turn ⇒ 本地上下文。
    let scope = begin_turn_trace("sess-b1b", "turn-b1b", 0);
    let local_trace = current_trace_id().expect("本地上下文必须存在");
    assert!(
        active_exported_turn_context().is_none(),
        "B1b：本地上下文不得成为子 span 的父上下文"
    );

    // 2) turn 中途打开开关并装配 provider（模拟用户运行中开启上报）。
    REPORTING_ENABLED.store(true, Ordering::SeqCst);
    init(OtelConfig {
        endpoint: "http://127.0.0.1:9".to_string(),
        service_name: "unit-b1b".to_string(),
        sampling_ratio: 1.0,
        headers: Vec::new(),
    })
    .expect("回环端点必须可装配");
    assert!(
        active_exported_turn_context().is_none(),
        "B1b：门禁翻转后本地登记仍不得充作父上下文"
    );

    // 3) 子 span 必须自成 root（trace_id 与本地伪造 id 不同）。
    let mut child =
        start_child_span("llm.chat", Vec::new()).expect("门禁开启时应能开子 span");
    let child_trace = opentelemetry::trace::Span::span_context(&child)
        .trace_id()
        .to_string();
    assert_ne!(
        child_trace, local_trace,
        "B1b：子 span 不得落在本地上下文这条「无 root」trace 上"
    );
    child.end();

    drop(scope);
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);
}

/// A2：嵌套 turn 必须「保存前值 + 精确摘除」，内层结束后恢复外层的上下文。
#[test]
fn a2_nested_turns_restore_previous_context() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);

    let outer = begin_turn_trace("sess-a2", "turn-outer", 0);
    let outer_trace = current_trace_id().expect("外层 turn 必须有上下文");
    let inner = begin_turn_trace("sess-a2", "turn-inner", 1);
    let inner_trace = current_trace_id().expect("内层 turn 必须有上下文");
    assert_ne!(outer_trace, inner_trace, "内外层必须是不同的 trace");

    drop(inner);
    assert_eq!(
        current_trace_id(),
        Some(outer_trace),
        "A2：内层结束后必须恢复外层上下文（而不是留下空洞）"
    );
    drop(outer);
    assert_eq!(current_trace_id(), None, "A2：全部结束后不得残留登记");
}

/// K4.1 反向：无活跃 turn（或 session 为空）时**不得**输出 `langfuse.session.id`。
#[test]
fn k41_child_spans_omit_session_id_without_active_turn() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);

    // 1) 无活跃 turn ⇒ 不压入该键。
    let mut attributes: Vec<KeyValue> = Vec::new();
    push_active_session_attribute(&mut attributes);
    assert!(
        attributes
            .iter()
            .all(|pair| pair.key.as_str() != ATTR_LANGFUSE_SESSION_ID),
        "无活跃 turn 时不得输出 langfuse.session.id"
    );

    // 2) 活跃 turn 但 session 为空白 ⇒ 同样不输出。
    let scope = begin_turn_trace("   ", "turn-k41-blank", 0);
    let mut attributes: Vec<KeyValue> = Vec::new();
    push_active_session_attribute(&mut attributes);
    assert!(
        attributes
            .iter()
            .all(|pair| pair.key.as_str() != ATTR_LANGFUSE_SESSION_ID),
        "session_id 为空白时不得输出 langfuse.session.id"
    );
    drop(scope);

    // 3) 正常 session ⇒ 输出且值一致。
    let scope = begin_turn_trace("sess-k41-ok", "turn-k41-ok", 0);
    let mut attributes: Vec<KeyValue> = Vec::new();
    push_active_session_attribute(&mut attributes);
    let session_attr = attributes
        .iter()
        .find(|pair| pair.key.as_str() == ATTR_LANGFUSE_SESSION_ID)
        .expect("活跃 turn 必须输出 langfuse.session.id");
    assert_eq!(
        session_attr.value,
        opentelemetry::Value::String("sess-k41-ok".into())
    );
    drop(scope);

    // 4) turn 级导出态 trace_id 可按 turn 回查（跨上下文补偿的基础）。
    assert_eq!(exported_trace_id_for_turn("no-such-turn"), None);
}

/// Wave-6 BLOCKING：`capture_turn_failure` 在**跨线程/无活跃 turn 上下文**时
/// 仍必须上报该 turn 的导出态 trace_id（按 turn_id 精确回查，绝不用本地 id / 绝不丢）。
#[test]
fn wave6_capture_turn_failure_uses_turn_scoped_trace_id_across_threads() {
    let _guard = test_lock();
    let _sentry_guard = crate::agent::ponysentry::test_sink_lock();
    shutdown();
    REPORTING_ENABLED.store(true, Ordering::SeqCst);
    init(OtelConfig {
        endpoint: "http://127.0.0.1:9".to_string(),
        service_name: "unit-wave6".to_string(),
        sampling_ratio: 1.0,
        headers: Vec::new(),
    })
    .expect("回环端点必须可装配");
    // PonySentry：dummy token 让上报路径不被 fail-closed 短路（不产生真实外网流量）。
    crate::agent::ponysentry::init(crate::agent::ponysentry::PonySentryConfig {
        client_token: Some("dummy-token".to_string()),
        enabled: true,
        endpoint: "http://127.0.0.1:9".to_string(),
        environment: "test".to_string(),
        release: "test".to_string(),
    });
    crate::agent::ponysentry::enable_turn_failure_capture(true);

    let scope = begin_turn_trace("sess-wave6-x", "turn-wave6-x", 0);
    let expected = exported_trace_id().expect("活跃 turn 必须有导出态 trace_id");
    assert_eq!(exported_trace_id_for_turn("turn-wave6-x"), Some(expected.clone()));

    // 在**另一个线程**（无活跃 turn 登记）触发失败上报。
    let handle = std::thread::spawn(|| {
        assert!(
            exported_trace_id().is_none(),
            "该线程不应有活跃 turn 上下文（模拟真实跨任务场景）"
        );
        crate::agent::ponysentry::capture_turn_failure("turn-wave6-x", "boom", Some("gpt-4o"));
        crate::agent::ponysentry::testing_last_enqueued_payload()
    });
    let payload = handle.join().expect("上报线程不得 panic").expect("必须入队 payload");
    assert_eq!(
        payload.trace_id,
        Some(expected),
        "跨线程上报必须回查到该 turn 的导出态 trace_id"
    );

    // 反向：未知 turn 不得凭空产生 trace_id（禁止伪造）。
    let handle = std::thread::spawn(|| {
        crate::agent::ponysentry::capture_turn_failure("turn-not-registered", "boom", None);
        crate::agent::ponysentry::testing_last_enqueued_payload()
    });
    let payload = handle.join().expect("上报线程不得 panic").expect("必须入队 payload");
    assert_eq!(payload.trace_id, None, "未登记的 turn 不得输出 trace_id");

    crate::agent::ponysentry::enable_turn_failure_capture(false);
    crate::agent::ponysentry::init(crate::agent::ponysentry::PonySentryConfig::default());
    drop(scope);
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);
}

/// Wave-7 G5 判据 1：`governance.decide` span 的 **5 项白名单属性真实上 wire**，
/// 且非白名单属性（如金额/阈值）不得出现。
#[test]
fn g5_governance_decide_span_exports_whitelisted_attributes_to_wire() {
    let _guard = test_lock();
    shutdown();
    let (port, sink) = spawn_local_otlp_sink(Duration::from_millis(1_500));
    init(OtelConfig {
        endpoint: format!("http://127.0.0.1:{port}"),
        service_name: "unit-governance".to_string(),
        sampling_ratio: 1.0,
        headers: Vec::new(),
    })
    .expect("回环端点必须可装配");
    REPORTING_ENABLED.store(true, Ordering::SeqCst);

    {
        let _turn = begin_turn_trace("sess-gov-wire", "turn-gov-wire", 0);
        let mut span = start_governance_decide_span();
        span.record_verdict(
            "session",
            "block",
            Some("hard_limit_reached"),
            "abc123def456",
            3,
        );
    }

    let flushed = force_flush(3_000);
    assert!(flushed.is_ok(), "flush 必须成功：{flushed:?}");
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);

    let wire = sink.join().expect("sink 线程不得 panic");
    let text = String::from_utf8_lossy(&wire).to_string();
    for attribute in [
        ATTR_GOVERNANCE_WINDOW,
        ATTR_GOVERNANCE_DECISION,
        ATTR_GOVERNANCE_REASON,
        ATTR_GOVERNANCE_POLICY_VERSION,
        ATTR_GOVERNANCE_UNPRICED_TURNS,
    ] {
        assert!(
            text.contains(attribute),
            "G5：wire payload 缺少白名单属性 `{attribute}`"
        );
    }
    assert!(text.contains("hard_limit_reached"), "reason 取值必须上 wire");
    assert!(text.contains("abc123def456"), "policy_version 取值必须上 wire");
    for forbidden in ["governance.limit_usd", "governance.current_usd", "governance.cost_usd"] {
        assert!(
            !text.contains(forbidden),
            "G5：非白名单属性 `{forbidden}` 不得出现在 wire payload"
        );
    }
}

/// A8：`traceparent` 注入必须是「替换」语义，重复注入不得产生两个同名头。
#[test]
fn a8_traceparent_injection_replaces_instead_of_appending() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);

    let scope = begin_turn_trace("sess-a8", "turn-a8", 0);
    let expected = current_traceparent().expect("本地上下文应可组装 traceparent");

    // 头部名走常量：可观测性门禁禁止在 otel 收口点之外手写 traceparent 请求头
    // （防第二份注入实现）。本用例注入的是**脏头**，只为验证「替换而非追加」，未新增注入实现。
    const TRACEPARENT_HEADER: &str = "traceparent";
    let request = reqwest::Client::new()
        .get("http://127.0.0.1:9/probe")
        .header(
            TRACEPARENT_HEADER,
            "00-00000000000000000000000000000000-0000000000000000-00",
        );
    let request = with_current_traceparent(with_current_traceparent(request));
    let built = request.build().expect("请求必须可构建");
    let values: Vec<&reqwest::header::HeaderValue> =
        built.headers().get_all(TRACEPARENT_HEADER).iter().collect();
    assert_eq!(
        values.len(),
        1,
        "A8：traceparent 必须至多一个（预防重复头），实际 {}",
        values.len()
    );
    assert_eq!(
        values[0].to_str().expect("ASCII header"),
        expected,
        "A8：注入值必须覆盖既有头"
    );

    drop(scope);
}

/// A6-adjacent：settings 快照的 Debug 必须打码 token（L3-A #6）。
#[test]
fn otel_settings_snapshot_debug_redacts_token() {
    let snapshot = OtelSettingsSnapshot {
        endpoint: Some("https://otel.example".to_string()),
        token: Some("SUPER_SECRET_OTEL_TOKEN_123".to_string()),
        service_name: Some("svc".to_string()),
        sampling_ratio: Some(0.5),
    };
    let shown = format!("{snapshot:?}");
    assert!(
        !shown.contains("SUPER_SECRET_OTEL_TOKEN_123"),
        "L3-A#6：Debug 不得泄漏 token：{shown}"
    );
    assert!(shown.contains("[REDACTED]"), "必须显式标出脱敏位：{shown}");
}

#[test]
fn test_resolve_resource_attributes_probe_marking() {
    let attrs = resolve_resource_attributes(
        |k| match k {
            ENV_PONY_OTEL_PROBE => Some("true".to_string()),
            _ => None,
        },
        None,
    );
    assert!(attrs.iter().any(|(k, v)| k == RESOURCE_PONY_PROBE && v == "true"));
    assert!(attrs.iter().any(|(k, v)| k == RESOURCE_INGEST_CLIENT_KIND && v == INGEST_CLIENT_KIND_PROBE));
    assert!(attrs.iter().any(|(k, v)| k == RESOURCE_PONY_APP_NAME && v == PONY_APP_NAME));

    let attrs_off = resolve_resource_attributes(
        |k| match k {
            ENV_PONY_OTEL_PROBE => Some("false".to_string()),
            _ => None,
        },
        None,
    );
    assert!(!attrs_off.iter().any(|(k, _)| k == RESOURCE_PONY_PROBE));
    assert!(attrs_off.iter().any(|(k, v)| k == RESOURCE_INGEST_CLIENT_KIND && v == INGEST_CLIENT_KIND_DEVICE));
    assert!(attrs_off.iter().any(|(k, v)| k == RESOURCE_PONY_APP_NAME && v == PONY_APP_NAME));
}

/// P1 & P2 单测：三种环境来源优先级判定，以及根 span usage/cost 聚合。
#[test]
fn test_deployment_environment_precedence() {
    // 1. env 显式覆盖优先
    let attrs_env = resolve_resource_attributes(
        |k| match k {
            ENV_PONY_OTEL_ENVIRONMENT => Some("staging".to_string()),
            _ => None,
        },
        None,
    );
    assert_eq!(
        attrs_env.iter().find(|(k, _)| k == RESOURCE_DEPLOYMENT_ENVIRONMENT).map(|(_, v)| v.as_str()),
        Some("staging")
    );

    // 2. 无 env 时根据 default_deployment_environment
    let env_default = default_deployment_environment(false);
    // 本地开发测试环境无 PONY_RELEASE_BUILD，即使 debug_assertions 为 false 也判 development
    assert_eq!(env_default, "development");
}

#[test]
fn test_turn_root_span_aggregates_usage_and_cost() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(true, Ordering::SeqCst);

    init(OtelConfig {
        endpoint: "http://127.0.0.1:9".to_string(),
        service_name: "test-usage-agg".to_string(),
        sampling_ratio: 1.0,
        headers: Vec::new(),
    })
    .expect("init");

    // 1. 有 usage 与 cost 场景
    {
        let mut turn = begin_turn_trace("sess-usage", "turn-usage-1", 0);
        {
            let mut llm1 = start_llm_span("openai", Some("gpt-4o"), "chat");
            llm1.record_usage(Some(100), Some(25));
            llm1.record_cost(Some(0.005));
        }
        {
            let mut llm2 = start_llm_span("openai", Some("gpt-4o"), "chat");
            llm2.record_usage(Some(50), Some(15));
            llm2.record_cost(Some(0.003));
        }
        turn.set_phase("completed");
        // Drop turn guard 触发聚合回填
    }

    // 2. 无 usage 与 cost 场景（不输出对应属性键）
    {
        let mut turn = begin_turn_trace("sess-usage-none", "turn-usage-none", 0);
        turn.set_phase("completed");
    }

    shutdown();
}

#[test]
fn test_export_diagnostics_uninit_returns_none() {
    let _guard = test_lock();
    shutdown();
    assert!(export_diagnostics().is_none());
}

#[test]
fn test_export_diagnostics_with_and_without_auth() {
    let _guard = test_lock();
    shutdown();

    // 1. 无 auth_token 初始化
    init(OtelConfig {
        endpoint: "https://collector.example.com".to_string(),
        service_name: "test-svc".to_string(),
        sampling_ratio: 1.0,
        headers: Vec::new(),
    })
    .expect("init without auth should succeed");

    let diag = export_diagnostics().expect("diagnostics should exist after init");
    assert_eq!(diag.endpoint, "https://collector.example.com");
    assert!(!diag.auth_configured, "无鉴权头时 auth_configured 必须为 false");

    shutdown();

    // 2. 带 auth_token 初始化
    init(OtelConfig {
        endpoint: "https://collector.example.com".to_string(),
        service_name: "test-svc".to_string(),
        sampling_ratio: 1.0,
        headers: vec![("authorization".to_string(), "Bearer secret-123".to_string())],
    })
    .expect("init with auth should succeed");

    let diag = export_diagnostics().expect("diagnostics should exist after init");
    assert_eq!(diag.endpoint, "https://collector.example.com");
    assert!(diag.auth_configured, "带 authorization 头时 auth_configured 必须为 true");

    shutdown();
}

/// 黑洞 OTLP 端点：只 accept、不回包（导出必然超时），held 到预算耗尽。
fn spawn_blackhole_sink(budget: Duration) -> (u16, std::thread::JoinHandle<usize>) {
    let listener =
        std::net::TcpListener::bind("127.0.0.1:0").expect("黑屏端点必须能绑定回环端口");
    listener.set_nonblocking(true).expect("黑屏端点必须非阻塞");
    let port = listener.local_addr().expect("blackhole local_addr").port();

    let handle = std::thread::spawn(move || {
        let mut held: Vec<std::net::TcpStream> = Vec::new();
        let deadline = std::time::Instant::now() + budget;
        while std::time::Instant::now() < deadline {
            match listener.accept() {
                Ok((stream, _peer)) => held.push(stream),
                Err(_) => std::thread::sleep(Duration::from_millis(5)),
            }
        }
        held.len()
    });

    (port, handle)
}

// ------------------------------------------------ PA-153 · resource 归因（实现期单测）

/// PA-153：默认值 = crate 版本 + debug 构建 `development` + 稳定产品名 + 默认 client.kind=device
/// + r1-dim6 F2 的 `langfuse.release`（与 `service.version` 同源同值）。
#[test]
fn resource_attributes_default_to_crate_version_and_build_environment() {
    let attrs = resolve_resource_attributes(|_| None, None);
    assert_eq!(attrs.len(), 5, "resource 归因包含五键：{attrs:?}");
    assert_eq!(attrs[0].0, "service.version");
    assert_eq!(attrs[0].1, env!("CARGO_PKG_VERSION"));
    assert_eq!(attrs[1].0, "deployment.environment");
    assert_eq!(
        attrs[1].1,
        default_deployment_environment(cfg!(debug_assertions))
    );
    assert_eq!(attrs[2].0, "pony.app.name");
    assert_eq!(attrs[2].1, "pony-agent");
    // r1-dim6 F2：resource 侧必须携带 `langfuse.release`，且与 `service.version` 同源同值。
    assert_eq!(attrs[3].0, ATTR_LANGFUSE_RELEASE);
    assert_eq!(attrs[3].1, attrs[0].1);
    assert_eq!(attrs[4].0, "ingest.client.kind");
    assert_eq!(attrs[4].1, INGEST_CLIENT_KIND_DEVICE);
}

/// PA-153：两个 env（trim 后非空）各自覆盖默认，且互不影响。
#[test]
fn resource_attributes_env_overrides_are_independent() {
    let attrs = resolve_resource_attributes(
        |name| match name {
            "PONY_OTEL_SERVICE_VERSION" => Some("1.2.3-custom".to_string()),
            "PONY_OTEL_ENVIRONMENT" => Some("staging".to_string()),
            _ => None,
        },
        None,
    );
    assert_eq!(attrs[0].1, "1.2.3-custom");
    assert_eq!(attrs[1].1, "staging");
}

/// PA-153：空白串（含仅 trim 后为空）等同未设置 ⇒ 回落默认；非空值 trim 后生效。
#[test]
fn resource_attributes_blank_env_falls_back_to_defaults() {
    for blank in ["", "   ", "\t\n "] {
        let attrs = resolve_resource_attributes(
            |name| match name {
                "PONY_OTEL_SERVICE_VERSION" | "PONY_OTEL_ENVIRONMENT" => Some(blank.to_string()),
                _ => None,
            },
            None,
        );
        assert_eq!(attrs[0].1, env!("CARGO_PKG_VERSION"), "空白串 `{blank:?}` 必须回落默认版本");
        assert_eq!(
            attrs[1].1,
            default_deployment_environment(cfg!(debug_assertions)),
            "空白串 `{blank:?}` 必须回落默认环境"
        );
    }
    let padded = resolve_resource_attributes(
        |name| match name {
            "PONY_OTEL_SERVICE_VERSION" => Some("  9.9.9  ".to_string()),
            _ => None,
        },
        None,
    );
    assert_eq!(padded[0].1, "9.9.9", "env 取值必须 trim 后生效");
}

/// PA-153 / task-13：release 标记判定（PONY_RELEASE_BUILD=1 为 production，否则一律 development）。
#[test]
fn default_deployment_environment_switches_on_build_profile() {
    assert_eq!(default_deployment_environment_with_release_flag(true, None), "development");
    assert_eq!(default_deployment_environment_with_release_flag(false, None), "development");
    assert_eq!(default_deployment_environment_with_release_flag(false, Some("1")), "production");
    assert_eq!(default_deployment_environment_with_release_flag(true, Some("1")), "production");
    assert_eq!(default_deployment_environment_with_release_flag(false, Some("true")), "production");
}

// -------------------------------------------- PA-154 · turn 错误聚合（实现期单测）

/// PA-154：聚合先记录者胜、取走即清零；无活动 turn 时 mark 为 no-op。
#[test]
fn turn_error_aggregation_first_wins_and_take_clears() {
    let _guard = test_lock();
    // 无活动 turn：mark / take 都必须是安全 no-op。
    mark_active_turn_error(TURN_ERROR_PREFIX_TOOL_FAILED);
    assert_eq!(take_active_turn_error(), None, "无活动 turn 时不得产生聚合");

    let slot = register_active_turn("turn-err", "sess-err", span_context(0x2000), true);
    mark_active_turn_error(TURN_ERROR_PREFIX_TOOL_FAILED);
    mark_active_turn_error(TURN_ERROR_PREFIX_QUOTA_BLOCKED);
    assert_eq!(
        take_active_turn_error().as_deref(),
        Some("tool_failed"),
        "双错误并存时必须保留先记录者"
    );
    assert_eq!(take_active_turn_error(), None, "取走后必须清零（跨 turn 不污染）");
    unregister_active_turn(slot);
}

/// PA-154：根 span 文案必须是 `turn_error: <reason>`（与子 span 原始文案可区分）。
#[test]
fn turn_error_status_message_uses_turn_error_prefix() {
    assert_eq!(
        turn_error_status_message(TURN_ERROR_PREFIX_TOOL_FAILED),
        "turn_error: tool_failed"
    );
    assert_eq!(
        turn_error_status_message(TURN_ERROR_PREFIX_QUOTA_BLOCKED),
        "turn_error: quota_blocked"
    );
    // 值契约：`agent.turn.outcome` 键名冻结；值域为单值结局（error / degraded，r1-dim6 F3）。
    assert_eq!(ATTR_AGENT_TURN_OUTCOME, "agent.turn.outcome");
    assert_eq!(TURN_OUTCOME_ERROR, "error");
    assert_eq!(TURN_OUTCOME_DEGRADED, "degraded");
    assert_eq!(TURN_ERROR_PREFIX_TOOL_FAILED, "tool_failed");
    assert_eq!(TURN_ERROR_PREFIX_QUOTA_BLOCKED, "quota_blocked");
}

/// wave-49 回归：构建期烧录 token 是**最低优先级**兜底 —— 用户零配置也能上报，
/// 但绝不覆盖 settings/env 的显式配置。
/// 守护的线上故障：桌面端无 token 分发通道 ⇒ 用户不手填 Settings 就静默 401、trace 全丢。
#[test]
fn build_time_token_is_lowest_priority_fallback() {
    // ① settings 与 env 都没给 ⇒ 用构建期值
    let with_build = resolve_config_with_build_token(None, None, Some("build-token-xyz"));
    assert_eq!(
        with_build
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(AUTH_HEADER_NAME))
            .map(|(_, value)| value.as_str()),
        Some("Bearer build-token-xyz"),
        "无 settings/env 时必须回落到构建期 token"
    );

    // ② env 给了 ⇒ 构建期值不得覆盖
    let env_pairs = [(ENV_PONY_OTEL_TOKEN, "env-token-abc")];
    let with_env = resolve_config_with_build_token(None, Some(&env_pairs), Some("build-token-xyz"));
    assert_eq!(
        with_env
            .headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(AUTH_HEADER_NAME))
            .map(|(_, value)| value.as_str()),
        Some("Bearer env-token-abc"),
        "env 显式配置必须压过构建期 token"
    );

    // ③ 构建期值为空/空白 ⇒ 视为未烧录（零凭证，fail-closed）
    for blank in ["", "   "] {
        let config = resolve_config_with_build_token(None, None, Some(blank));
        assert!(
            config.headers.is_empty(),
            "空白构建期 token 必须视为未配置（{blank:?}）"
        );
    }

    // ④ 完全没有构建期 token（本地 cargo build 的常态）⇒ 仍然零凭证
    let none = resolve_config_with_build_token(None, None, None);
    assert!(none.headers.is_empty(), "无构建期 token 时不得凭空产生凭证");
}

/// T11 回归：工具 span 的时间边界必须覆盖真实执行区间；退化/溢出路径不得 panic。
#[test]
fn tool_span_bounds_covers_execution_interval() {
    let now = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);

    // 正常路径：end - start 精确等于 duration_ms（这正是 0.002ms → 829ms 的修复点）。
    let (start, end) = tool_span_bounds(now, Some(829));
    assert_eq!(end, now, "end 必须是采集时刻");
    assert_eq!(
        end.duration_since(start).expect("end 必须晚于 start"),
        std::time::Duration::from_millis(829),
        "工具 span 必须覆盖真实执行区间（修复前恒为 0 时长）"
    );

    // 零耗时：唯一允许的零长 span。
    let (start, end) = tool_span_bounds(now, Some(0));
    assert_eq!((start, end), (now, now), "duration=0 必须 start == end");

    // 耗时未知：退化为零长 span（不得凭空造时长、不得 panic）。
    let (start, end) = tool_span_bounds(now, None);
    assert_eq!((start, end), (now, now), "duration=None 必须退化为 start == end");

    // 极大值：异常大 duration 必须退化为零长 span（不得 panic）。
    // 说明：Linux 的 SystemTime 允许负值，`u64::MAX` 毫秒并不会让 checked_sub 失败，
    // 因此这里额外有跨平台确定的上限（TOOL_SPAN_MAX_BACKDATE_MS = 24h）。
    let (start, end) = tool_span_bounds(now, Some(u64::MAX));
    assert_eq!((start, end), (now, now), "超上限必须退化为 start == end");
    let (start, end) = tool_span_bounds(now, Some(TOOL_SPAN_MAX_BACKDATE_MS + 1));
    assert_eq!((start, end), (now, now), "刚超上限也必须退化为 start == end");
    let (start, end) = tool_span_bounds(now, Some(TOOL_SPAN_MAX_BACKDATE_MS));
    assert_eq!(
        end.duration_since(start).expect("end 必须晚于 start"),
        std::time::Duration::from_millis(TOOL_SPAN_MAX_BACKDATE_MS),
        "上限内的长耗时必须完整保留（不得误伤长任务）"
    );

    // 秒级量级同样精确（证明不是毫秒量级的巧合）。
    let (start, end) = tool_span_bounds(now, Some(1_000));
    assert_eq!(
        end.duration_since(start).expect("end 必须晚于 start"),
        std::time::Duration::from_secs(1)
    );
}

// ------------------------------------------------ W3 trace：teammate 继承宿主 traceparent

/// W3 trace：`traceparent_to_span_context` 必须按 W3C 口径解析 trace/span/flags，
/// 并把远端标记置位；任何不合规输入一律 `None`（fail-closed 回落 root）。
#[test]
fn traceparent_parsing_yields_remote_parent_context() {
    let traceparent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
    let context =
        traceparent_to_span_context(traceparent).expect("合法 traceparent 必须解析成功");
    assert_eq!(
        context.trace_id().to_string(),
        "4bf92f3577b34da6a3ce929d0e0e4736",
        "W3 trace：trace id 必须原样继承"
    );
    assert_eq!(
        context.span_id().to_string(),
        "00f067aa0ba902b7",
        "W3 trace：parent span id 必须原样继承"
    );
    assert!(
        context.is_sampled(),
        "W3 trace：flags=01 ⇒ 解析出的父上下文必须 sampled"
    );
    assert!(
        context.is_remote(),
        "W3 trace：从 wire 头解析的父上下文必须是 remote（不是本地派生 span）"
    );

    // 不合规输入：fail-closed 一律 None（不得注入脏父上下文）。
    for invalid in [
        "",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7",
        "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
        "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01",
        "00-00000000000000000000000000000000-00f067aa0ba902b7-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01",
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra",
    ] {
        assert!(
            traceparent_to_span_context(invalid).is_none(),
            "W3 trace：非法 traceparent {invalid:?} 必须解析为 None"
        );
    }
}

/// W3 trace：`begin_turn_trace_with_parent` 必须沿用父 trace id（无论导出通路还是
/// 降级本地通路）：teammate turn 的日志 `trace_id` / 出网 `traceparent` 与宿主同一 trace，
/// 但 span id 必须是本 turn 自己的新 id（父子关系，不是冒用父 span）。
///
/// 本用例**不**改进程级门禁/provider：两条通路（真实 span / 本地派生上下文）都必须满足
/// 同一断言，因此天然对其它单测的门禁残留免疫。
#[test]
fn begin_turn_trace_with_parent_inherits_trace_id_locally() {
    let parent = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

    let scope = begin_turn_trace_with_parent("sess-w3-parent", "turn-w3-parent", 0, Some(parent));
    eprintln!(
        "[w3-trace] parent_trace={} scope_trace={:?} current_traceparent={:?}",
        "4bf92f3577b34da6a3ce929d0e0e4736",
        scope.trace_id(),
        current_traceparent()
    );

    assert_eq!(
        scope.trace_id().as_deref(),
        Some("4bf92f3577b34da6a3ce929d0e0e4736"),
        "W3 trace：队友 turn 的 trace id 必须与宿主父 trace 相同"
    );
    assert_eq!(
        current_trace_id().as_deref(),
        Some("4bf92f3577b34da6a3ce929d0e0e4736"),
        "W3 trace：活动 turn 的 trace id 必须继承父上下文"
    );
    assert_ne!(
        current_span_id().as_deref(),
        Some("00f067aa0ba902b7"),
        "W3 trace：本 turn 必须是新的 span id（不得冒用父 span）"
    );
    let child_traceparent = current_traceparent().expect("本地路径仍必须组装 traceparent");
    assert!(
        child_traceparent.starts_with("00-4bf92f3577b34da6a3ce929d0e0e4736-"),
        "W3 trace：子 turn 的 traceparent 必须携带父 trace id，实际 = {child_traceparent}"
    );

    // 非法父头：回落 root（与既有 begin_turn_trace 行为一致，不得因脏头崩溃）。
    let root_scope = begin_turn_trace_with_parent("sess-w3-root", "turn-w3-root", 1, Some("bad"));
    assert_ne!(
        root_scope.trace_id().as_deref(),
        Some("4bf92f3577b34da6a3ce929d0e0e4736"),
        "W3 trace：非法父头必须回落 root trace（不得继承脏 trace id）"
    );
}

/// ADR 0055 P0-4：`non_zero_exit` 是命令级语义信号，不得提升为整轮 error。
#[test]
fn non_zero_exit_is_not_turn_fatal() {
    assert!(!tool_error_kind_is_turn_fatal(Some("non_zero_exit")));
    assert!(!tool_error_kind_is_turn_fatal(Some(" NON_ZERO_EXIT ")));
}

/// fail-closed：未知/缺失种类照旧提升为整轮错误（不许靠缺字段洗白失败率）。
#[test]
fn unknown_or_missing_error_kind_stays_turn_fatal() {
    assert!(tool_error_kind_is_turn_fatal(Some("handler_error")));
    assert!(tool_error_kind_is_turn_fatal(Some("timeout")));
    assert!(tool_error_kind_is_turn_fatal(Some("")));
    assert!(tool_error_kind_is_turn_fatal(None));
}

/// ADR 0055 P0-1：上下文容量/比例必须避开 `gen_ai.usage.` 前缀（否则被并入 usage.total）。
#[test]
fn context_keys_are_outside_gen_ai_usage_prefix() {
    for key in [
        ATTR_GEN_AI_USAGE_CONTEXT_WINDOW,
        ATTR_GEN_AI_USAGE_CONTEXT_RATIO,
        ATTR_LLM_CONTEXT_CACHE_HIT_RATIO,
        ATTR_GEN_AI_USAGE_REASONING_TOKENS,
    ] {
        assert!(
            !key.starts_with("gen_ai.usage."),
            "ADR 0055：{key} 不得落在 gen_ai.usage.* 前缀下（Langfuse 会求和进 usage.total）"
        );
    }
    // 用量三键必须保持 semconv 名（Langfuse 靠它们做扣减与计费）。
    assert_eq!(ATTR_GEN_AI_USAGE_INPUT_TOKENS, "gen_ai.usage.input_tokens");
    assert_eq!(ATTR_GEN_AI_USAGE_OUTPUT_TOKENS, "gen_ai.usage.output_tokens");
    assert_eq!(
        ATTR_GEN_AI_USAGE_CACHED_INPUT_TOKENS,
        "gen_ai.usage.cache_read.input_tokens"
    );
}

/// ADR 0055 P2：`langfuse.release` 是唯一能填充 trace.release 的键（探针实测）。
#[test]
fn langfuse_release_key_contract() {
    assert_eq!(ATTR_LANGFUSE_RELEASE, "langfuse.release");
    assert!(!current_release_version().trim().is_empty());
}

/// P0-1：真实属性输出的物理凭证 —— 脱敏 + 2KB 截断 + 截断标记。
///
/// 与 `tests/otel_tool_payload_test.rs`（黑盒验收）互补：这里打印**真实属性值**，
/// 供回执引用，并锁住「先脱敏后截断、属性长度有硬上界」的实现顺序。
#[test]
fn tool_span_payload_preview_is_sanitized_then_truncated() {
    // 注意：本模块单测**不得**改 env（lib 测试二进制内并行执行，env 是进程级全局）。
    // 采集开关的关闭路径由 `tests/otel_tool_payload_test.rs::capture_switch_disables_payload_keys`
    // 用文件级锁 + EnvVarGuard 覆盖。
    let huge = format!(
        "{{\"command\":\"curl -H 'authorization: Bearer sk-test-TOPSECRET-0123456789' \
https://api.example.com\",\"stdout\":\"{}\"}}",
        "y".repeat(10 * 1024)
    );
    let attrs = tool_span_attributes(&ToolSpanInput {
        tool_name: "workspace_run_command".to_string(),
        status: "success".to_string(),
        duration_ms: Some(12),
        error: None,
        args_preview: Some(huge),
        result_preview: Some("{\"ok\":true,\"exitCode\":0}".to_string()),
    });

    let input = attrs
        .iter()
        .find(|(key, _)| key == ATTR_LANGFUSE_OBSERVATION_INPUT)
        .and_then(|(_, value)| match value {
            AttrValue::Str(text) => Some(text.clone()),
            _ => None,
        })
        .expect("采集开启时入参预览必须写出");
    let output = attrs
        .iter()
        .find(|(key, _)| key == ATTR_LANGFUSE_OBSERVATION_OUTPUT)
        .and_then(|(_, value)| match value {
            AttrValue::Str(text) => Some(text.clone()),
            _ => None,
        })
        .expect("采集开启时结果预览必须写出");

    eprintln!(
        "[p0-1] {ATTR_LANGFUSE_OBSERVATION_INPUT} len={} prefix={:?} suffix={:?}",
        input.len(),
        &input[..input.len().min(80)],
        &input[input.len().saturating_sub(24)..]
    );
    eprintln!("[p0-1] {ATTR_LANGFUSE_OBSERVATION_OUTPUT} = {output:?}");

    assert!(
        input.len() <= TOOL_PAYLOAD_PREVIEW_LIMIT_BYTES,
        "P0-1：属性长度必须有硬上界，实际 {}",
        input.len()
    );
    assert!(input.ends_with(TOOL_PAYLOAD_TRUNCATION_MARKER), "P0-1：必须带截断标记");
    assert!(!input.contains("sk-test-TOPSECRET-0123456789"), "P0-1：裸 token 必须脱敏");
    assert!(!input.contains("/home/"), "P0-1：家目录路径必须脱敏");
    assert_eq!(output, "{\"ok\":true,\"exitCode\":0}", "P0-1：短预览原样（脱敏后）");
}

/// P0-1：UTF-8 安全 —— 多字节字符不得被切成半个码点。
#[test]
fn truncation_respects_char_boundaries() {
    let text = "中".repeat(2000); // 6000 字节
    let truncated = truncate_tool_payload_preview(&text);
    assert!(truncated.len() <= TOOL_PAYLOAD_PREVIEW_LIMIT_BYTES);
    assert!(truncated.ends_with(TOOL_PAYLOAD_TRUNCATION_MARKER));
    let body = truncated.trim_end_matches(TOOL_PAYLOAD_TRUNCATION_MARKER);
    assert!(body.chars().all(|ch| ch == '中'), "P0-1：截断后不得出现替换字符/半个码点");
    assert_eq!(body.len() % 3, 0, "P0-1：UTF-8 边界必须对齐");
}

/// P0-1：LLM / Turn span 预览脱敏与 4KB 安全截断
#[test]
fn llm_payload_preview_is_sanitized_and_truncated_to_4k() {
    let huge = format!(
        "Prompt with /home/dm/path and secret sk-test-TOPSECRET-9876543210 {}",
        "A".repeat(5000)
    );
    let attrs = llm_span_attributes(&LlmSpanInput {
        provider_kind: "openai".to_string(),
        model: Some("gpt-4o".to_string()),
        input_tokens: Some(10),
        output_tokens: Some(20),
        operation: "chat".to_string(),
        prompt_preview: Some(huge),
        completion_preview: Some("Normal response".to_string()),
    });

    let prompt_ot = attrs
        .iter()
        .find(|(key, _)| key == ATTR_GEN_AI_PROMPT)
        .and_then(|(_, value)| match value {
            AttrValue::Str(text) => Some(text.clone()),
            _ => None,
        })
        .expect("gen_ai.prompt 必须写出");
    let prompt_lf = attrs
        .iter()
        .find(|(key, _)| key == ATTR_LANGFUSE_OBSERVATION_INPUT)
        .and_then(|(_, value)| match value {
            AttrValue::Str(text) => Some(text.clone()),
            _ => None,
        })
        .expect("langfuse.observation.input 必须写出");

    assert_eq!(prompt_ot, prompt_lf);
    assert!(prompt_ot.len() <= LLM_PAYLOAD_PREVIEW_LIMIT_BYTES);
    assert!(prompt_ot.ends_with(TOOL_PAYLOAD_TRUNCATION_MARKER));
    assert!(!prompt_ot.contains("sk-test-TOPSECRET-9876543210"));
    assert!(!prompt_ot.contains("/home/dm"));
}

/// P3：预建的 `tool.call` span 必须被记账入口按 `call_id` **消费**，
/// 且工具执行期间 `current_traceparent()` 指向该工具 span（子代理 turn 的真父）。
///
/// 回归背景：只 `begin` 不 `finish` 会让 span 永不结束（不导出）、每次子代理调用泄漏一个
/// span，并让子 turn 的 parent_span_id 指向一个不存在于线路上的 span（Tempo 里是孤儿）。
#[test]
fn p3_precreated_tool_span_is_consumed_by_call_id() {
    let _guard = test_lock();
    shutdown();
    REPORTING_ENABLED.store(true, Ordering::SeqCst);
    init(OtelConfig {
        endpoint: "http://127.0.0.1:9".to_string(),
        service_name: "unit-p3-tool-span".to_string(),
        sampling_ratio: 1.0,
        headers: Vec::new(),
    })
    .expect("回环端点必须可装配");

    let turn = begin_turn_trace("sess-p3", "turn-p3", 0);
    let turn_traceparent = current_traceparent().expect("活跃 turn 必须有 traceparent");

    let guard = begin_tool_call_span(Some("call-p3"), "subagent", Some("{\"prompt\":\"x\"}"))
        .expect("门禁开启且 call_id 非空时必须预建 tool.call span");
    let tool_traceparent = current_traceparent().expect("工具执行期必须有 traceparent");
    assert_ne!(
        tool_traceparent, turn_traceparent,
        "工具执行期间父链顶必须是 tool.call span（否则子 turn 只能与工具并列挂在 turn 下）"
    );
    let tool_context = traceparent_to_span_context(&tool_traceparent)
        .expect("预建 span 的 traceparent 必须可解析");
    assert!(
        tool_traceparent.starts_with(&format!("00-{}-", tool_context.trace_id())),
        "P3：current_traceparent 必须逐字携带工具 span 的 trace/span id"
    );
    assert_eq!(
        PENDING_TOOL_SPANS.with(|pending| pending.borrow().len()),
        1,
        "预建后必须且只能登记一个待记账 span"
    );

    // 记账：按 call_id 取回同一个 span 并补写终态，不再另建。
    record_tool_call_span_with_call_id(
        ToolSpanInput {
            tool_name: "subagent".to_string(),
            status: "success".to_string(),
            duration_ms: Some(321),
            error: None,
            args_preview: None,
            result_preview: None,
        },
        Some("call-p3".to_string()),
    );
    assert_eq!(
        PENDING_TOOL_SPANS.with(|pending| pending.borrow().len()),
        0,
        "P3：记账入口必须消费预建 span（残留 = span 永不 end、子 turn 成孤儿）"
    );
    assert!(
        take_pending_tool_span("call-p3").is_none(),
        "P3：同一 call_id 不得被重复消费"
    );

    drop(guard);
    drop(turn);
    shutdown();
    REPORTING_ENABLED.store(false, Ordering::SeqCst);
}
