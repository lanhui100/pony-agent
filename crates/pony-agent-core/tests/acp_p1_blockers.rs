//! ACP P1 阻断项（Gate-1/Gate-2 BLOCKING）回归红相（B150-P1-T 补充，task-6 后新增文件）。
//!
//! 两个阻断事实（Lead 2026-11-14 裁决，不改已冻结的 4 个测试文件）：
//! - **P1-9 预算不被 5s 硬顶**：`session/prompt` 的 deadline 必须取
//!   `AcpAgentSession::set_turn_budget`（`client.set_prompt_budget` 的当前值），
//!   默认值 `ACP_REQUEST_TIMEOUT` 只是缺省，不是不可突破的上限。
//! - **P1-10 取消令牌联动**：prompt 运行期间宿主取消 `CancellationToken` ⇒ 会话必须发
//!   `session/cancel` notification，并把本轮归约为 `status: aborted` + `tool_cancelled`，
//!   不得等满整轮预算、不得 panic。
//!
//! 纪律：阈值全部取自 `.dev-team/nfr-baseline.json` 的 `acp` 段；注入延迟 = baseline + margin
//! （margin 为 Lead 冻结的 2000ms / 3000ms）；等待用有界条件轮询 / `recv_timeout`，无 `#[ignore]`。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::protocol::ContentBlock;
use pony_agent_core::acp::session::{AcpAgentSession, ACP_TOOL_CANCELLED};
use pony_agent_core::acp::transport::ACP_REQUEST_TIMEOUT;
use pony_agent_core::agent::budget::CancellationToken;
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::turn_flow::TurnEventSink;
use serde_json::Value;

// ───────────────────────────── 设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-p1b-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    path.canonicalize().expect("canonicalize 临时工作目录")
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// NFR 唯一真源。
fn nfr_acp_ms(key: &str) -> u64 {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.dev-team/nfr-baseline.json");
    let body = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读取 NFR 真源失败 {}: {error}", path.display()));
    let parsed: Value = serde_json::from_str(&body).expect("nfr-baseline.json 必须是合法 JSON");
    parsed
        .get("acp")
        .and_then(|acp| acp.get(key))
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("nfr-baseline.json 缺 acp.{key}"))
}

fn fake_spec(mode: &str, extra: &[&str]) -> AcpAgentSpec {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    let mut args = vec![script.to_string_lossy().into_owned(), mode.to_string()];
    args.extend(extra.iter().map(|value| (*value).to_string()));
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

fn spawn_session(mode: &str, extra: &[&str], cwd: &Path) -> AcpAgentSession {
    match AcpAgentSession::spawn(&fake_spec(mode, extra), cwd) {
        Ok(session) => session,
        Err(error) => panic!("AcpAgentSession::spawn 失败（fake agent 应可用）: {error:?}"),
    }
}

/// 有界条件轮询（到达条件即返回）。
fn wait_until<F: FnMut() -> bool>(mut condition: F, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    condition()
}

/// 本文件不校验 UI 事件，仅吞掉 update（行为等价于无观测）。
struct NoopSink;

impl TurnEventSink for NoopSink {
    fn emit(&self, _name: &str, _payload: TurnStreamEvent) {}
}

fn prompt_blocks(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

// ───────────────────────── P1-9 整轮预算不得被 5s 硬顶 ─────────────────────────

/// P1-9：agent 延迟 = `ACP_REQUEST_TIMEOUT + 2000ms`（NFR 派生），
/// `set_turn_budget(20s)` 后本轮必须成功返回 `ok`/end_turn（而不是 `acp_timeout`）。
#[test]
fn p1_09_turn_budget_is_not_capped_by_request_timeout() {
    let baseline_ms = nfr_acp_ms("request_timeout_ms");
    assert_eq!(
        baseline_ms,
        ACP_REQUEST_TIMEOUT.as_millis() as u64,
        "P1-9 前置：NFR 基准必须等于 ACP_REQUEST_TIMEOUT 常量"
    );
    // Lead 冻结注入值：延迟 = baseline + 2000ms；预算 20s（= 4 × baseline，NFR 派生）。
    let delay_ms = baseline_ms + 2000;
    let budget_ms = baseline_ms * 4;
    assert_eq!(budget_ms, 20_000, "P1-9：整轮预算口径必须为 20s（Lead 冻结）");
    let delay_seconds = format!("{:.3}", delay_ms as f64 / 1000.0);

    let cwd = temp_dir("budget");
    let mut session = spawn_session(
        "slow_prompt",
        &[delay_seconds.as_str(), "acp-sess-1"],
        &cwd,
    );
    session.set_turn_budget(Duration::from_millis(budget_ms));

    let started = Instant::now();
    let result = session.prompt(prompt_blocks("slow task"), &NoopSink);
    let elapsed = started.elapsed();

    assert_eq!(
        result.status, "ok",
        "P1-9 违约：{delay_ms}ms 的一轮在 {budget_ms}ms 预算下必须成功（不得被 {}ms 硬顶成 acp_timeout），实际 {:?}，耗时 {elapsed:?}",
        baseline_ms, result
    );
    assert!(
        elapsed >= Duration::from_millis(delay_ms),
        "P1-9：agent 确实延迟了 {delay_ms}ms，实测耗时 {elapsed:?}（否则本用例没有覆盖 5s 边界）"
    );
    assert!(
        elapsed < Duration::from_millis(budget_ms),
        "P1-9 违约：必须在预算 {budget_ms}ms 内返回，实测 {elapsed:?}"
    );

    session.close();
    cleanup(&cwd);
}

// ───────────────────────── P1-10 取消令牌联动 ─────────────────────────

/// P1-10：prompt 运行期触发 `CancellationToken` ⇒ 会话必须发 `session/cancel` notification
/// （fake agent 收到才回包，并落 `<marker>.cancel` 证据），并把本轮归约为
/// `aborted` + `tool_cancelled`；不得等满整轮预算（baseline + 3000ms）、不得 panic。
#[test]
fn p1_10_cancellation_token_aborts_prompt_and_notifies_agent() {
    let baseline_ms = nfr_acp_ms("request_timeout_ms");
    // cancel_hold 模式在收到 session/cancel 前永不回包 ⇒ 其等价「延迟上界」即本轮预算。
    let horizon_ms = baseline_ms + 3000;
    assert!(
        horizon_ms > baseline_ms,
        "P1-10：预算必须大于 5s 基准，才能证明取消独立于预算生效"
    );

    let cwd = temp_dir("cancel-token");
    let marker = cwd.join("prompt-received.marker");
    let marker_text = marker.to_string_lossy().into_owned();
    let session = spawn_session(
        "cancel_hold",
        &[marker_text.as_str(), "acp-sess-1"],
        &cwd,
    );
    session.set_turn_budget(Duration::from_millis(horizon_ms));

    let token = Arc::new(CancellationToken::new());
    session.set_cancellation_token(Arc::clone(&token));

    let (sender, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = session.prompt(prompt_blocks("long running task"), &NoopSink);
            let _ = sender.send(result);
        });

        assert!(
            wait_until(|| marker.exists(), Duration::from_millis(baseline_ms)),
            "P1-10 前置：fake agent 必须在 {}ms 内收到 session/prompt",
            baseline_ms
        );

        let cancelled_at = Instant::now();
        token.cancel();

        let result = receiver
            .recv_timeout(Duration::from_millis(horizon_ms))
            .expect("P1-10 违约：取消后 prompt 必须及时返回，实际等满预算（取消未联动）");
        let elapsed = cancelled_at.elapsed();

        assert_eq!(
            result.status, "aborted",
            "P1-10 违约：取消归约状态必须是 aborted，实际 {result:?}"
        );
        assert!(
            result.output.contains(ACP_TOOL_CANCELLED),
            "P1-10 违约：取消归约错误码必须是 {ACP_TOOL_CANCELLED}，实际 {}",
            result.output
        );
        assert!(
            elapsed < Duration::from_millis(horizon_ms),
            "P1-10 违约：取消必须在预算内生效，实测 {elapsed:?}（预算 {horizon_ms}ms）"
        );

        // 取消链路证据：agent 只有在真正收到 session/cancel notification 后才会回包并落盘该文件。
        let cancel_evidence = cwd.join("prompt-received.marker.cancel");
        assert!(
            wait_until(|| cancel_evidence.exists(), Duration::from_millis(baseline_ms)),
            "P1-10 违约：会话未向 agent 发送 session/cancel notification（agent 侧无证据）"
        );
        let raw = std::fs::read_to_string(&cancel_evidence).expect("读取 cancel 证据");
        let frame: Value =
            serde_json::from_str(raw.lines().next().expect("cancel 证据行缺失")).expect("cancel 证据必须是 JSON");
        assert_eq!(
            frame["method"],
            serde_json::json!("session/cancel"),
            "P1-10：发出的必须是 session/cancel notification"
        );
    });

    let mut session = session;
    session.close();
    cleanup(&cwd);
}
