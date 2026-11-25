//! ACP cancel-race 归约契约（backlog B169，Gate-1 r4 / Gate-2 r4 ADVISORY 收口）。
//!
//! 冻结的事实：`AcpAgentSession::prompt` 曾经只在 `Err` 分支看取消事实——agent 若在收到
//! `session/cancel` 后仍以**成功** stopReason（`end_turn` / `max_tokens`）收尾，本轮会被
//! 归约成 `ok`，界面上"已取消的一轮"变成"成功的一轮"。生产路径靠 dispatcher 兜住，
//! 但会话层契约本身不成立。
//!
//! 本文件冻结两条边界（无 `#[ignore]`、无空断言）：
//! - **R1（红相）**：取消已触发 + agent 仍回 `end_turn` ⇒ `status: aborted` +
//!   `tool_cancelled`，且 `stopReason` 归约为 `cancelled`；
//! - **R2（负向对照）**：未取消的正常轮次**仍**是 `ok`（不得为了堵 R1 把成功轮一起打成 aborted）。
//!
//! 纪律：夹具走 `tests/fixtures/acp_fake_agent.sh`；R1 用 `slow_prompt`（**故意不理**
//! `session/cancel`，延迟后回 `end_turn`），延迟/预算取自 `.dev-team/nfr-baseline.json` 的
//! `acp` 段派生，不写裸魔法数。

#[path = "support/paths.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::protocol::ContentBlock;
use pony_agent_core::acp::session::{AcpAgentSession, ACP_TOOL_CANCELLED};
use pony_agent_core::agent::budget::CancellationToken;
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::tools::ToolResult;
use pony_agent_core::agent::turn_flow::TurnEventSink;
use serde_json::Value;

// ───────────────────────────── 设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-cancelrace-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    support::deverbatim(&path.canonicalize().expect("canonicalize 临时工作目录"))
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// NFR 唯一真源（与 `acp_p1_blockers.rs` 同口径，不复制数值）。
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
        inherit_proxy_env: true,
        mcp_servers: None,
    }
}

fn spawn_session(mode: &str, extra: &[&str], cwd: &Path) -> AcpAgentSession {
    match AcpAgentSession::spawn(&fake_spec(mode, extra), cwd) {
        Ok(session) => session,
        Err(error) => panic!("AcpAgentSession::spawn 失败（fake agent 应可用）: {error:?}"),
    }
}

/// 本文件只校验终局归约，不校验 UI 事件序列。
struct NoopSink;

impl TurnEventSink for NoopSink {
    fn emit(&self, _name: &str, _payload: TurnStreamEvent) {}
}

fn prompt_blocks(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

/// 结果 JSON 里的 `error.code`（成功面为 `None`）。
fn error_code(result: &ToolResult) -> Option<String> {
    let parsed: Value =
        serde_json::from_str(&result.output).expect("acp_delegate 结果必须是合法 JSON");
    parsed
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn stop_reason(result: &ToolResult) -> Option<String> {
    let parsed: Value =
        serde_json::from_str(&result.output).expect("acp_delegate 结果必须是合法 JSON");
    parsed
        .get("stopReason")
        .and_then(Value::as_str)
        .map(str::to_string)
}

// ───────────────────────────── R1 红相 ─────────────────────────────

/// R1：取消已触发，但 agent 不理 `session/cancel`、延迟后回 `end_turn` ⇒ 本轮必须归约
/// `aborted` + `tool_cancelled`（取消事实优先于 stopReason），且**确实是等到了 agent 收尾**
/// 才归约（`duration_ms ≥ 延迟 - 容差`，否则说明走的是"prompt 入口即取消"的旁路，
/// 没有覆盖 Ok 分支）。
#[test]
fn r1_cancel_race_end_turn_is_reduced_to_aborted() {
    let baseline_ms = nfr_acp_ms("request_timeout_ms");
    // 延迟 = 基准的一半（≥1s，确保取消一定落在 prompt 进行中；< 基准，确保不触发传输层超时）。
    let delay_ms = baseline_ms / 2;
    assert!(
        delay_ms >= 1_000 && delay_ms < baseline_ms,
        "R1：延迟必须落在 (1s, {baseline_ms}ms) 内，实际 {delay_ms}ms"
    );
    // 整轮预算 = 基准 × 2（NFR 派生），保证延迟不会撞预算。
    let budget_ms = baseline_ms * 2;
    let delay_seconds = format!("{:.3}", delay_ms as f64 / 1000.0);

    let cwd = temp_dir("cancel-race");
    let session = spawn_session(
        "slow_prompt",
        &[delay_seconds.as_str(), "acp-sess-1"],
        &cwd,
    );
    session.set_turn_budget(Duration::from_millis(budget_ms));
    let token = Arc::new(CancellationToken::new());
    session.set_cancellation_token(Arc::clone(&token));

    let cancel_after = Duration::from_millis(200);
    let (sender, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = session.prompt(prompt_blocks("cancel race"), &NoopSink);
            let _ = sender.send(result);
        });

        // 握手在 `spawn` 内已完成，prompt 线程此刻必然已发起 `session/prompt`；
        // 取消落在 agent 的延迟窗口内 ⇒ 本用例覆盖的正是 Ok 分支的 cancel-race。
        std::thread::sleep(cancel_after);
        token.cancel();
    });

    let result = receiver
        .recv_timeout(Duration::from_millis(budget_ms + nfr_acp_ms("spawn_reap_grace_ms") * 2))
        .expect("R1：prompt 必须在预算内返回（不得挂起）");

    assert_eq!(
        result.status, "aborted",
        "R1 违约：取消已触发，即使 agent 回 end_turn 也必须归约 aborted，实际 {:?}，输出 {}",
        result.status, result.output
    );
    assert_eq!(
        error_code(&result).as_deref(),
        Some(ACP_TOOL_CANCELLED),
        "R1 违约：取消归约的错误码必须为 {ACP_TOOL_CANCELLED}，实际输出 {}",
        result.output
    );
    assert_eq!(
        stop_reason(&result).as_deref(),
        Some("cancelled"),
        "R1 违约：归约后的 stopReason 必须是 cancelled（不得沿用 agent 的 end_turn），实际输出 {}",
        result.output
    );
    let agent_delay_ms = delay_ms.saturating_sub(cancel_after.as_millis() as u64);
    assert!(
        result.duration_ms >= agent_delay_ms,
        "R1 前置失守：本轮耗时 {}ms 小于 agent 收尾前的剩余延迟 {}ms ⇒ 说明取消在 prompt \
         入口即生效、没有走到 Ok 分支，本用例未覆盖 cancel-race",
        result.duration_ms,
        agent_delay_ms
    );

    let mut session = session;
    session.close();
    cleanup(&cwd);
}

// ───────────────────────────── R2 负向对照 ─────────────────────────────

/// R2：未取消的轮次**仍**归约 `ok`（R1 的收口不得把正常成功轮打坏）。
#[test]
fn r2_uncancelled_turn_stays_ok() {
    let cwd = temp_dir("no-cancel");
    let session = spawn_session("slow_prompt", &["0.300", "acp-sess-1"], &cwd);
    session.set_turn_budget(Duration::from_millis(nfr_acp_ms("request_timeout_ms") * 2));

    let result = session.prompt(prompt_blocks("no cancel"), &NoopSink);
    assert_eq!(
        result.status, "ok",
        "R2 违约：未取消的轮次必须仍是 ok，实际 {:?}，输出 {}",
        result.status, result.output
    );
    assert!(
        error_code(&result).is_none(),
        "R2 违约：成功轮不得带 error.code，实际输出 {}",
        result.output
    );
    assert_eq!(
        stop_reason(&result).as_deref(),
        Some("end_turn"),
        "R2 违约：成功轮的 stopReason 必须沿用 agent 的 end_turn，实际输出 {}",
        result.output
    );

    let mut session = session;
    session.close();
    cleanup(&cwd);
}
