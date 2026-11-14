//! ACP P0 L2-AT 对抗破坏契约（B149-P0-T，冻结于 task-2）。
//!
//! 三类注入（真源 §6 风险 + `.dev-team/contract-matrix-acp-p0.md` A7/A8/A9）：
//! 1. 畸形输入：单行非法跳过不崩（< 阈值）；连续非法达阈值 fail-closed（非 panic）；
//! 2. 超时穿透：注入延迟 = baseline_timeout + margin，deadline 内必须返回 `acp_timeout`；
//! 3. 确定性并发：固定 id 集合 + `Barrier` 同步 + 固定轮次，断言返回结果与各自 sentinel 一一对应。
//! 另含：取消通知必须无 `id`（A2）、子进程崩溃返回错误而非 panic。
//!
//! 确定性纪律：全部阈值来自 `.dev-team/nfr-baseline.json` 的 `acp` 段；
//! 等待使用有界条件轮询 / `Barrier`，不使用「纯 sleep + 概率」作为同步手段。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Barrier};
use std::time::{Duration, Instant};

use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::protocol::method;
use pony_agent_core::acp::protocol::{
    ClientCapabilities, InitializeRequest, InitializeResponse, Implementation, PROTOCOL_VERSION,
};
use pony_agent_core::acp::transport::{
    AcpTransport, AcpTransportError, MALFORMED_LINE_STREAK_LIMIT, ACP_REQUEST_TIMEOUT,
    SPAWN_REAP_GRACE, STDERR_TAIL_LINES,
};
use serde_json::{json, Value};

// ───────────────────────────── 通用设施（与蓝军文件保持独立，避免跨 target 共享） ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-p0-at-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    path.canonicalize().expect("canonicalize 临时工作目录")
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// NFR 唯一真源（禁在本文件自选阈值）。
fn nfr_json() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.dev-team/nfr-baseline.json");
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 NFR 真源失败 {}: {error}", path.display()));
    serde_json::from_str(&body).expect("nfr-baseline.json 必须是合法 JSON")
}

fn nfr_acp_ms(key: &str) -> u64 {
    nfr_json()
        .get("acp")
        .and_then(|acp| acp.get(key))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("nfr-baseline.json 缺 acp.{key}（task-1 NFR 段未就绪）"))
}

fn fake_spec(mode: &str, arg: Option<&str>) -> AcpAgentSpec {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    let mut args = vec![script.to_string_lossy().into_owned(), mode.to_string()];
    if let Some(extra) = arg {
        args.push(extra.to_string());
    }
    AcpAgentSpec {
        id: "fake".to_string(),
        label: "Fake ACP agent".to_string(),
        command: "sh".to_string(),
        args,
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: AcpCapabilitiesHint::default(),
    }
}

fn initialize_params() -> Value {
    serde_json::to_value(InitializeRequest {
        protocol_version: PROTOCOL_VERSION,
        client_capabilities: ClientCapabilities::default(),
        client_info: Some(Implementation {
            name: "pony-agent-tests".to_string(),
            title: None,
            version: "0.0.0".to_string(),
        }),
    })
    .expect("initialize 请求必须可序列化")
}

fn spawn(spec: &AcpAgentSpec, cwd: &Path) -> AcpTransport {
    match AcpTransport::spawn(spec, cwd, &[]) {
        Ok(transport) => transport,
        Err(error) => panic!("fake agent 应可启动，实际 {error:?}"),
    }
}

/// 注入延迟 = baseline_timeout + margin；margin 由 NFR 基线派生（timeout 的 10%），非自选常数。
fn timeout_margin_ms(timeout_ms: u64) -> u64 {
    assert!(timeout_ms >= 1000, "NFR request_timeout_ms 至少应为 1s，实际 {timeout_ms}");
    timeout_ms / 10
}

// ───────────────────────────── 1. 畸形输入（A8） ─────────────────────────────

/// 连续非法行 < 阈值：单行非法必须被跳过（记诊断），请求不得失败、不得 panic。
#[test]
fn adversarial_malformed_below_threshold_is_skipped() {
    let limit = nfr_acp_ms("malformed_line_streak_limit");
    assert_eq!(
        limit, MALFORMED_LINE_STREAK_LIMIT as u64,
        "A8：阈值真源与代码常量必须一致"
    );
    assert!(limit >= 2, "单行容忍用例要求阈值 >= 2，实际 {limit}");

    let cwd = temp_dir("malformed-single");
    let spec = fake_spec("malformed_single", None);
    let mut transport = spawn(&spec, &cwd);

    let result = transport
        .request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT)
        .expect("阈值以下的单行非法 JSON 必须被跳过，不能拖垮请求（更不得 panic）");
    let parsed: InitializeResponse =
        serde_json::from_value(result).expect("跳过非法行后仍必须收到合规 initialize result");
    assert_eq!(parsed.protocol_version, 1);

    transport.close();
    cleanup(&cwd);
}

/// 连续非法行 == 阈值：必须 fail-closed 返回 `acp_protocol_error`，且不等到 deadline、不 panic。
#[test]
fn adversarial_malformed_streak_fail_closed() {
    let limit = nfr_acp_ms("malformed_line_streak_limit");
    let streak = limit.to_string();

    let cwd = temp_dir("malformed-streak");
    let spec = fake_spec("malformed_streak", Some(&streak));
    let mut transport = spawn(&spec, &cwd);

    let started = Instant::now();
    let error = match transport.request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT) {
        Ok(result) => panic!("连续 {limit} 行非法 JSON 必须 fail-closed，实际返回 {result}"),
        Err(error) => error,
    };
    let elapsed = started.elapsed();

    assert!(
        matches!(error, AcpTransportError::Protocol(_)),
        "A8 违约：连续非法行必须返回 Protocol 错误（acp_protocol_error），实际 {error:?}"
    );
    assert_eq!(error.code(), "acp_protocol_error", "A8：错误码必须是 acp_protocol_error");
    assert!(
        elapsed < ACP_REQUEST_TIMEOUT,
        "A8 违约：fail-closed 必须立即生效，不能挂到 deadline（耗时 {elapsed:?}）"
    );

    transport.close();
    cleanup(&cwd);
}

// ───────────────────────────── 2. 超时穿透（A7） ─────────────────────────────

/// agent 延迟 = baseline_timeout + margin：request 必须在 deadline 内返回 `acp_timeout`。
#[test]
fn adversarial_request_timeout_within_deadline() {
    let timeout_ms = nfr_acp_ms("request_timeout_ms");
    assert_eq!(
        timeout_ms,
        ACP_REQUEST_TIMEOUT.as_millis() as u64,
        "A7：超时真源与代码常量必须一致"
    );
    let margin_ms = timeout_margin_ms(timeout_ms);
    let delay_ms = timeout_ms + margin_ms;
    let delay_seconds = format!("{:.3}", delay_ms as f64 / 1000.0);

    let cwd = temp_dir("timeout");
    let spec = fake_spec("timeout", Some(&delay_seconds));
    let mut transport = spawn(&spec, &cwd);

    let started = Instant::now();
    let outcome = transport.request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT);
    let elapsed = started.elapsed();

    let error = match outcome {
        Ok(result) => panic!("agent 延迟 {delay_ms}ms 超过 deadline {timeout_ms}ms，绝不能成功: {result}"),
        Err(error) => error,
    };
    assert_eq!(
        error.code(),
        "acp_timeout",
        "A7 违约：超过 deadline 必须返回 acp_timeout，实际 {error:?}"
    );
    assert!(
        matches!(error, AcpTransportError::Timeout(_)),
        "A7 违约：超时必须落到 Timeout 变体，实际 {error:?}"
    );
    // 下界容差同样由 NFR 派生（timeout - margin = baseline 的 90%）：既防止「过早返回」蒙混，
    // 也不因调度抖动产生 flaky
    let lower_bound_ms = timeout_ms - margin_ms;
    assert!(
        elapsed >= Duration::from_millis(lower_bound_ms),
        "A7：超时不得早于 deadline 触发（耗时 {elapsed:?}，下界 {lower_bound_ms}ms）"
    );
    assert!(
        elapsed < Duration::from_millis(delay_ms),
        "A7：必须在 agent 回包之前按 deadline 返回（耗时 {elapsed:?}，回包 {delay_ms}ms）"
    );
    // 上限容差取 NFR 的 spawn 回收宽限，不引入自选数值
    assert!(
        elapsed < Duration::from_millis(timeout_ms + 4 * nfr_acp_ms("spawn_reap_grace_ms")),
        "A7：超时反馈时延不得失控（耗时 {elapsed:?}）"
    );

    transport.close();
    cleanup(&cwd);
}

// ───────────────────────────── 3. 确定性并发（id 不串包） ─────────────────────────────

/// 固定 id 集合 + Barrier 同步 + 固定轮次：并发调用方的结果必须与本方 sentinel 严格对应。
#[test]
fn adversarial_concurrent_requests_do_not_cross_ids() {
    const THREADS: usize = 8;
    const ROUNDS: usize = 12;

    let cwd = temp_dir("concurrent");
    let spec = fake_spec("concurrent", None);
    let transport = Arc::new(spawn(&spec, &cwd));
    let barrier = Arc::new(Barrier::new(THREADS));

    let mut handles = Vec::with_capacity(THREADS);
    for thread in 0..THREADS {
        let transport = Arc::clone(&transport);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            // Barrier 同步：所有线程在同一时刻开始冲击同一 transport（无 sleep 竞态）
            barrier.wait();
            for round in 0..ROUNDS {
                let sentinel = format!("T{thread}-R{round}");
                let result = transport
                    .request(
                        "fake/ping",
                        json!({"sentinel": sentinel.clone()}),
                        ACP_REQUEST_TIMEOUT,
                    )
                    .unwrap_or_else(|error| panic!("并发请求 {sentinel} 失败: {error:?}"));
                assert_eq!(
                    result["echo"].as_str(),
                    Some(sentinel.as_str()),
                    "并发 id 串包：请求 {sentinel} 收到了 {result}"
                );
            }
        }));
    }
    for handle in handles {
        handle.join().expect("并发工作线程不得 panic");
    }

    let mut transport = match Arc::try_unwrap(transport) {
        Ok(transport) => transport,
        Err(_) => panic!("所有工作线程结束后 transport 应唯一持有"),
    };
    transport.close();
    cleanup(&cwd);
}

// ───────────────────────────── 4. 取消通知无 id（A2） ─────────────────────────────

/// `session/cancel` 是 notification：报文不得带 `id`（否则 agent 会等应答）。
#[test]
fn adversarial_cancel_notification_has_no_id() {
    let cwd = temp_dir("cancel");
    let capture = cwd.join("captured-cancel.jsonl");
    let spec = fake_spec("capture", Some(capture.to_string_lossy().as_ref()));
    let mut transport = spawn(&spec, &cwd);

    transport
        .request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT)
        .expect("initialize 应成功");
    transport
        .notify(method::SESSION_CANCEL, json!({"sessionId": "s-1"}))
        .expect("session/cancel notification 必须可发送");
    // 顺序性验证请求：fake agent 按 stdin 行序处理，故此时 cancel 行必已被捕获
    transport
        .request("fake/verify", json!({"ok": true}), ACP_REQUEST_TIMEOUT)
        .expect("cancel 后 transport 仍必须可用");

    let raw = std::fs::read_to_string(&capture).expect("fake agent 必须捕获 session/cancel 原始行");
    let lines: Vec<&str> = raw.lines().filter(|line| !line.trim().is_empty()).collect();
    assert_eq!(lines.len(), 1, "session/cancel 必须恰好发送一次，实际 {raw:?}");
    let frame: Value = serde_json::from_str(lines[0])
        .unwrap_or_else(|error| panic!("cancel 报文必须是合法 JSON: {error}\n{}", lines[0]));
    assert_eq!(frame["jsonrpc"], json!("2.0"), "JSON-RPC 2.0 信封缺失");
    assert_eq!(frame["method"], json!(method::SESSION_CANCEL));
    assert!(
        frame.get("id").is_none(),
        "A2 违约：session/cancel 是 notification，不得携带 id，实际 {}",
        lines[0]
    );
    assert_eq!(
        frame["params"]["sessionId"],
        json!("s-1"),
        "cancel params 必须携带 sessionId"
    );

    transport.close();
    cleanup(&cwd);
}

// ───────────────────────────── 5. 子进程崩溃 → 错误而非 panic ─────────────────────────────

/// agent 启动即崩溃：必须返回传输层错误（含 spawn/protocol/unavailable），禁止 panic 或挂死。
#[test]
fn adversarial_crashed_agent_yields_error_not_panic() {
    let cwd = temp_dir("crash");
    let spec = fake_spec("crash", None);
    let mut transport = spawn(&spec, &cwd);

    let started = Instant::now();
    let error = match transport.request(method::INITIALIZE, initialize_params(), ACP_REQUEST_TIMEOUT) {
        Ok(result) => panic!("已崩溃的 agent 不得返回成功结果: {result}"),
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            AcpTransportError::Spawn(_)
                | AcpTransportError::Protocol(_)
                | AcpTransportError::Unavailable(_)
        ),
        "崩溃必须归约为传输层错误，实际 {error:?}"
    );
    assert!(
        started.elapsed() < ACP_REQUEST_TIMEOUT,
        "崩溃必须立即暴露，不能挂到 deadline"
    );

    transport.close();
    cleanup(&cwd);
}

/// stderr 上限常量参与对抗断言（确保 A8/A9 相邻常量未被静默改动）。
#[test]
fn adversarial_stderr_tail_cap_is_frozen() {
    assert_eq!(STDERR_TAIL_LINES, 32);
    assert!(SPAWN_REAP_GRACE >= Duration::from_millis(100));
}
