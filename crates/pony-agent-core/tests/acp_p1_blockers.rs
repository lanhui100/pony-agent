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
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::protocol::ContentBlock;
use pony_agent_core::acp::session::{AcpAgentSession, ACP_TOOL_CANCELLED};
use pony_agent_core::acp::transport::ACP_REQUEST_TIMEOUT;
use pony_agent_core::agent::budget::CancellationToken;
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
use pony_agent_core::agent::turn_flow::TurnEventSink;
use serde_json::{json, Value};

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

fn fixture_script() -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script.to_string_lossy().into_owned()
}

/// 进程级 XDG 隔离的串行闸（本文件仅 P1-11 需要读 acp-agents.json）。
static CONFIG_ENV_LOCK: Mutex<()> = Mutex::new(());

fn write_acp_agents_config(config_home: &Path, spec: Value) {
    let dir = config_home.join("pony-agent");
    std::fs::create_dir_all(&dir).expect("创建 pony-agent 配置目录");
    std::fs::write(
        dir.join("acp-agents.json"),
        serde_json::to_string_pretty(&json!([spec])).expect("清单必须可序列化"),
    )
    .expect("写入 acp-agents.json");
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

// ───────────────────────── P1-11 工具级取消联动（生产路径一半） ─────────────────────────

/// P1-11：`acp_delegate` 必须把 `ToolExecutionContext.cancellation` 接入 ACP 会话，
/// 使 stop_turn 取消能真正杀掉外部 agent：
/// - fake agent 延迟 = baseline + 3000ms（后台 sleep 代表长任务，期间可接收 `session/cancel`）；
/// - 运行中取消后，`execute_acp_delegate` 必须 `status=aborted` + `code=tool_cancelled`；
/// - 耗时必须显著小于 slow 延迟；
/// - agent 侧必须出现 `<marker>.cancel` 证据（证明 `session/cancel` 真发出）。
#[test]
fn p1_11_tool_path_cancellation_aborts_acp_delegate() {
    let baseline_ms = nfr_acp_ms("request_timeout_ms");
    let delay_ms = baseline_ms + 3000;
    let delay_seconds = format!("{:.3}", delay_ms as f64 / 1000.0);

    let cwd = temp_dir("tool-cancel");
    let config_home = cwd.join("config");
    let marker = cwd.join("tool-prompt.marker");
    write_acp_agents_config(
        &config_home,
        json!({
            "id": "fake",
            "label": "Fake ACP agent",
            "command": "sh",
            "args": [
                fixture_script(),
                "slow_cancel",
                marker.to_string_lossy(),
                delay_seconds,
                "acp-sess-1"
            ]
        }),
    );

    let _env_guard = CONFIG_ENV_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let old_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("XDG_CONFIG_HOME", &config_home);

    let router = ToolRouter::with_workspace_root(cwd.clone());
    let token = Arc::new(CancellationToken::new());
    let context = ToolExecutionContext {
        workspace_root: Some(cwd.clone()),
        session_id: Some("tool-session".to_string()),
        cancellation: Some(Arc::clone(&token)),
        ..Default::default()
    };
    let call = ToolCall {
        call_id: Some("call-acp-1".to_string()),
        name: "acp_delegate".to_string(),
        arguments: json!({
            "agent": "fake",
            "prompt": "long running external task",
            "timeoutMs": delay_ms,
            "description": "委派给外部 ACP agent"
        }),
        plan: None,
    };

    let (sender, receiver) = mpsc::channel();
    let (cancel_to_result, result) = std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = router.execute_with_context(&call, &context);
            let _ = sender.send(result);
        });

        assert!(
            wait_until(|| marker.exists(), Duration::from_millis(baseline_ms)),
            "P1-11 前置：fake agent 必须在 {baseline_ms}ms 内收到 prompt"
        );
        let cancelled_at = Instant::now();
        token.cancel();
        let result = receiver
            .recv_timeout(Duration::from_millis(delay_ms + baseline_ms))
            .expect("P1-11 违约：取消后 acp_delegate 必须及时返回（不得等满 slow 延迟/预算）");
        (cancelled_at.elapsed(), result)
    });

    match old_xdg {
        Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }

    assert_eq!(
        result.tool_name, "acp_delegate",
        "P1-11：结果必须来自 acp_delegate，实际 {result:?}"
    );
    assert_eq!(
        result.status, "aborted",
        "P1-11 违约：工具级取消必须归约 status=aborted（当前未接线时会是 error/acp_timeout），实际 {result:?}"
    );
    assert!(
        result.output.contains(ACP_TOOL_CANCELLED),
        "P1-11 违约：错误码必须是 {ACP_TOOL_CANCELLED}，实际 {}",
        result.output
    );
    assert!(
        cancel_to_result < Duration::from_millis(delay_ms),
        "P1-11 违约：取消必须在 slow 延迟 {delay_ms}ms 之前生效，实测 {cancel_to_result:?}"
    );

    // 取消链路证据：agent 只有在真正收到 session/cancel 后才落盘该文件。
    let evidence = cwd.join("tool-prompt.marker.cancel");
    assert!(
        wait_until(|| evidence.exists(), Duration::from_millis(baseline_ms)),
        "P1-11 违约：未向外部 agent 发送 session/cancel notification（agent 侧无证据）"
    );
    let raw = std::fs::read_to_string(&evidence).expect("读取 cancel 证据");
    let frame: Value = serde_json::from_str(raw.lines().next().expect("cancel 证据行缺失"))
        .expect("cancel 证据必须是 JSON");
    assert_eq!(
        frame["method"],
        json!("session/cancel"),
        "P1-11：发出的必须是 session/cancel notification"
    );

    cleanup(&cwd);
}

// ───────────────────────── P1-12 CLI 走线（--timeout-secs 真进预算） ─────────────────────────
//
// [POST-HOC-GREEN: 实现先于用例，仅作回归守护]
// 背景：Gate-1 r2 指出「红相只直调库 setter，绿 ≠ 交付可用」。本条改走**真实交付入口**
// （`ponycli acp` 子进程），证明 `--timeout-secs` 确实进入整轮预算：
// - 延迟 = baseline + 1500ms（> 5s 硬顶、< 30s 预算）→ EXIT 0 且 stdout 含 `turn:delta`；
// - 同延迟、`--timeout-secs 1` → EXIT 7（Timeout 档，usage 契约）。

use std::process::{Command, Output, Stdio};

fn cli_timeout_for(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

/// 有界执行 ponycli（stdin=null 防 REPL 挂死；超时 kill 并附原始输出）。
fn run_ponycli(command: &mut Command, timeout: Duration) -> Output {
    command.stdin(Stdio::null());
    let label = format!("{command:?}");
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("ponycli 必须可执行（CARGO_BIN_EXE_ponycli）");
    let started = Instant::now();
    loop {
        match child.try_wait().expect("try_wait ponycli") {
            Some(_) => break,
            None => {
                if started.elapsed() >= timeout {
                    let _ = child.kill();
                    let output = child.wait_with_output().expect("收集超时输出");
                    panic!(
                        "CLI 未在 {timeout:?} 内退出\ncmd: {label}\nstdout:\n{}\nstderr:\n{}",
                        String::from_utf8_lossy(&output.stdout),
                        String::from_utf8_lossy(&output.stderr)
                    );
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
    child.wait_with_output().expect("收集 ponycli 输出")
}

/// 隔离的 `ponycli acp` 入口（XDG_CONFIG_HOME 注入 fake spec；HOME/XDG_* 全隔离）。
struct AcpCliEnv {
    root: PathBuf,
}

impl AcpCliEnv {
    fn new(tag: &str, spec: Value) -> Self {
        let root = temp_dir(tag);
        for child in ["home", "config", "data", "state", "cache", "workspace"] {
            std::fs::create_dir_all(root.join(child)).expect("创建隔离子目录");
        }
        write_acp_agents_config(&root.join("config"), spec);
        Self { root }
    }

    fn workspace(&self) -> PathBuf {
        self.root.join("workspace")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ponycli"));
        command
            .env("HOME", self.root.join("home"))
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_DATA_HOME", self.root.join("data"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_CACHE_HOME", self.root.join("cache"));
        command
    }
}

impl Drop for AcpCliEnv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn p1_12_cli_timeout_secs_feeds_turn_budget() {
    let baseline_ms = nfr_acp_ms("request_timeout_ms");
    let delay_ms = baseline_ms + 1500; // > 5s 硬顶、< 30s 预算（Lead 冻结注入值）
    let delay_seconds = format!("{:.3}", delay_ms as f64 / 1000.0);
    let env = AcpCliEnv::new(
        "cli-budget",
        json!({
            "id": "fake",
            "label": "Fake ACP agent",
            "command": "sh",
            "args": [fixture_script(), "slow_prompt", delay_seconds, "acp-sess-1"]
        }),
    );
    let workspace = env.workspace();
    let workspace_text = workspace.to_string_lossy().into_owned();
    let watchdog = cli_timeout_for(delay_ms + 2 * baseline_ms);

    // (a) 30s 预算：延迟 6.5s 的一轮必须成功，且 jsonl stdout 事件流含 turn:delta。
    let ok_run = run_ponycli(
        env.command().args([
            "acp",
            "--agent",
            "fake",
            "-p",
            "hi",
            "--output-format",
            "jsonl",
            "--workspace",
            workspace_text.as_str(),
            "--timeout-secs",
            "30",
        ]),
        watchdog,
    );
    let stdout = String::from_utf8_lossy(&ok_run.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&ok_run.stderr).into_owned();
    assert_eq!(
        ok_run.status.code(),
        Some(0),
        "P1-12 违约：{}ms 的一轮在 --timeout-secs 30 下必须 EXIT 0（不得被 5s 硬顶）\nstdout:\n{stdout}\nstderr:\n{stderr}",
        delay_ms
    );
    let mut saw_delta = false;
    for line in stdout.lines().filter(|line| !line.trim().is_empty()) {
        let value: Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!("P1-12 违约：jsonl stdout 每行必须是合法 JSON（{error}）：{line}")
        });
        if value["type"] == json!("turn:delta") && value["text"] == json!("hello") {
            saw_delta = true;
        }
    }
    assert!(
        saw_delta,
        "P1-12 违约：CLI 事件流必须含 turn:delta(text=hello)（证明真实入口把 update 映射进 JsonlSink）\nstdout:\n{stdout}"
    );

    // (b) 1s 预算：同延迟必须 EXIT 7（Timeout 档），不得塌进 0/5。
    let timeout_run = run_ponycli(
        env.command().args([
            "acp",
            "--agent",
            "fake",
            "-p",
            "hi",
            "--output-format",
            "jsonl",
            "--workspace",
            workspace_text.as_str(),
            "--timeout-secs",
            "1",
        ]),
        watchdog,
    );
    assert_eq!(
        timeout_run.status.code(),
        Some(7),
        "P1-12 违约：--timeout-secs 1 必须 EXIT 7（Timeout），证明该参数真进预算\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&timeout_run.stdout),
        String::from_utf8_lossy(&timeout_run.stderr)
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&timeout_run.stdout),
        String::from_utf8_lossy(&timeout_run.stderr)
    );
    assert!(
        !combined.contains("panicked at"),
        "P1-12：超时路径不得 panic，实际输出：\n{combined}"
    );
}

// ───────────────────────── P1-13 宿主取消令牌只读 ─────────────────────────
//
// [POST-HOC-GREEN: 实现先于用例，仅作回归守护]
// 契约：`session.cancel()` 是「宿主 → 外部 agent」的出站取消，**不得**把宿主自己的
// `CancellationToken` 置位（否则会污染整个 turn 的取消语义）；同时会话内挂起等待必须归零
// （prompt 归约 aborted / tool_cancelled，不得悬挂）。
#[test]
fn p1_13_session_cancel_does_not_mutate_host_token() {
    let baseline_ms = nfr_acp_ms("request_timeout_ms");
    let horizon_ms = baseline_ms + 3000;

    let cwd = temp_dir("host-token");
    let marker = cwd.join("prompt-received.marker");
    let marker_text = marker.to_string_lossy().into_owned();
    let session = spawn_session("cancel_hold", &[marker_text.as_str(), "acp-sess-1"], &cwd);
    session.set_turn_budget(Duration::from_millis(horizon_ms));

    let host_token = Arc::new(CancellationToken::new());
    assert!(!host_token.is_cancelled(), "P1-13 前置：宿主令牌初始必须未取消");
    session.set_cancellation_token(Arc::clone(&host_token));

    let (sender, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = session.prompt(prompt_blocks("long running task"), &NoopSink);
            let _ = sender.send(result);
        });

        assert!(
            wait_until(|| marker.exists(), Duration::from_millis(baseline_ms)),
            "P1-13 前置：fake agent 必须在 {baseline_ms}ms 内收到 session/prompt"
        );

        session
            .cancel()
            .expect("P1-13：session.cancel() 必须成功发出 session/cancel notification");

        let result = receiver
            .recv_timeout(Duration::from_millis(horizon_ms))
            .expect("P1-13 违约：cancel 后挂起等待必须归零（prompt 未返回）");
        assert_eq!(
            result.status, "aborted",
            "P1-13 违约：会话内挂起等待必须归约 aborted，实际 {result:?}"
        );
        assert!(
            result.output.contains(ACP_TOOL_CANCELLED),
            "P1-13 违约：错误码必须是 {ACP_TOOL_CANCELLED}，实际 {}",
            result.output
        );

        assert!(
            !host_token.is_cancelled(),
            "P1-13 违约：session.cancel() 不得把宿主 CancellationToken 置位（出站取消与宿主生命周期必须解耦）"
        );
    });

    let mut session = session;
    session.close();
    cleanup(&cwd);
}
