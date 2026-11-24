//! ACP 配置/能力回归红相（review `.dev-team/review-acp-config.json` 的 4 条 BLOCKING）。
//!
//! 覆盖：
//! - **h1** `terminal/create → wait_for_exit → output` 标准序下 output 必须仍含全部 stdout
//!   （实现若在 wait 里 drain 缓冲 ⇒ output 为空，红）；
//! - **h2** workspace 存在未信任 `.mcp.json` 时 inherit 必须成功并忽略该配置（只记诊断），
//!   不得 `acp_config_invalid`（红）；
//! - **h3** 握手声明 caps 全 false 的会话，事后 `install_fs_handler` 不得让未声明能力追认：
//!   agent 的 `fs/*` 必须被拒（JSON-RPC error），不得真实读文件（红）；
//! - **h4** 长命令（sleep 6s）的 `wait_for_exit` 必须在 NFR 派生预算内返回（不得 5s 硬顶），
//!   且同轮后续入站请求仍被应答（读线程不饿死）。
//!
//! 纪律：只碰 ACP 测试与夹具；阈值取自 `.dev-team/nfr-baseline.json` 的 `acp` 段 /
//! `ACP_DEFAULT_TURN_BUDGET`；无 `#[ignore]`、无空断言。

#[path = "support/paths.rs"]
mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pony_agent_core::acp::config::{
    resolve_mcp_servers_in, AcpAgentSpec, AcpCapabilitiesHint, AcpEnvVariable,
};
use pony_agent_core::acp::handlers::{ManagedTerminalHandler, WorkspaceFsHandler};
use pony_agent_core::acp::protocol::ContentBlock;
use pony_agent_core::acp::session::AcpAgentSession;
use pony_agent_core::acp::transport::{AcpTransportError, ACP_HANDSHAKE_TIMEOUT};
use pony_agent_core::acp::ACP_DEFAULT_TURN_BUDGET;
use pony_agent_core::agent::path_permission::{AuthorizeStore, PathPurpose};
use pony_agent_core::agent::process::ProcessManager;
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::turn_flow::TurnEventSink;
use serde_json::{json, Value};

// ───────────────────────────── 设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-regr-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    support::deverbatim(&path.canonicalize().expect("canonicalize 临时工作目录"))
}

fn fixture_script() -> PathBuf {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script
}

fn hint(fs: bool, terminal: bool) -> AcpCapabilitiesHint {
    AcpCapabilitiesHint { fs, terminal }
}

fn spec_with_hint(hint: AcpCapabilitiesHint, mode: &str, extra: &[&str]) -> AcpAgentSpec {
    let mut args = vec![fixture_script().to_string_lossy().into_owned(), mode.to_string()];
    args.extend(extra.iter().map(|value| (*value).to_string()));
    AcpAgentSpec {
        id: "fake".to_string(),
        label: "Fake ACP agent".to_string(),
        command: "sh".to_string(),
        args,
        env: Vec::new(),
        default_cwd: None,
        capabilities_hint: hint,
        inherit_proxy_env: true,
        // Some([]) ⇒ 显式禁用 inherit（h2 例外：需要 inherit 路径时单独构造）。
        mcp_servers: Some(Vec::new()),
    }
}

fn captured_responses(cwd: &Path) -> Vec<Value> {
    let path = cwd.join("responses.jsonl");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fake agent 必须捕获入站应答（{}：{error}）", path.display()));
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str::<Value>(line).expect("应答行必须是合法 JSON"))
        .collect()
}

fn response_for(responses: &[Value], id: u64) -> Value {
    responses
        .iter()
        .find(|response| response["id"].as_u64() == Some(id))
        .unwrap_or_else(|| panic!("缺少 id={id} 的应答，实际 {responses:?}"))
        .clone()
}

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

struct NoopSink;
impl TurnEventSink for NoopSink {
    fn emit(&self, _name: &str, _payload: TurnStreamEvent) {}
}

fn run_prompt(session: &AcpAgentSession) -> Duration {
    let started = Instant::now();
    let result = session.prompt(
        vec![ContentBlock::Text {
            text: "probe".to_string(),
        }],
        &NoopSink,
    );
    let elapsed = started.elapsed();
    assert_eq!(result.status, "ok", "probe 轮必须正常结束（end_turn），实际 {result:?}");
    elapsed
}

fn workspace_fs_handler(root: &Path) -> Arc<WorkspaceFsHandler> {
    Arc::new(WorkspaceFsHandler::new(
        root.to_path_buf(),
        Arc::new(AuthorizeStore::new()),
        PathPurpose::Read,
    ))
}

fn managed_terminal_handler() -> Arc<ManagedTerminalHandler> {
    Arc::new(ManagedTerminalHandler::new(Arc::new(ProcessManager::new())))
}

fn write_paths_file(cwd: &Path, paths: &[&str]) -> PathBuf {
    let path = cwd.join("probe-paths.jsonl");
    let body: String = paths.iter().map(|value| format!("{value}\n")).collect();
    std::fs::write(&path, body).expect("写 paths 文件");
    path
}

// ───────────────────────── h1 wait→output 顺序 ─────────────────────────

/// h1：`create → wait_for_exit → output` 下 output 必须仍含全部 stdout；
/// 实现若在 wait 阶段 drain 缓冲 ⇒ output 为空（当前红）。
#[test]
fn h1_output_after_wait_for_exit_keeps_stdout() {
    let cwd = temp_dir("wait-then-output");
    let marker = format!("pony-h1-{}-{}", std::process::id(), Instant::now().elapsed().as_nanos());
    let marker_file = cwd.join("marker.txt");
    std::fs::write(&marker_file, &marker).expect("写 marker 文件");
    let init_file = cwd.join("initialize.jsonl");
    let resp_file = cwd.join("responses.jsonl");
    let spec = spec_with_hint(
        hint(false, true),
        "probe_terminal_wait_first",
        &[
            init_file.to_string_lossy().as_ref(),
            resp_file.to_string_lossy().as_ref(),
            marker_file.to_string_lossy().as_ref(),
        ],
    );

    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        None,
        Some(managed_terminal_handler()),
    ) {
        Ok(session) => session,
        Err(error) => panic!("h1：spawn 必须成功：{error:?}"),
    };
    run_prompt(&session);

    let responses = captured_responses(&cwd);
    let waited = response_for(&responses, 9302);
    assert_eq!(
        waited["result"]["exitCode"],
        json!(0),
        "h1 前置：wait_for_exit 必须成功归约 exitCode=0，实际 {waited}"
    );
    let output = response_for(&responses, 9303);
    let text = output["result"]["output"]
        .as_str()
        .unwrap_or_else(|| panic!("h1 违约：wait 之后的 output 必须返回 result，实际 {output}"));
    assert!(
        text.contains(&marker),
        "h1 违约：create→wait_for_exit→output 标准序下 stdout 不得被 wait 读走丢弃；\
         期望含 `{marker}`，实际 output={text:?}"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── h2 未信任 .mcp.json 不得致命 ─────────────────────────

/// h2：workspace 存在 `.mcp.json`（未设 `PONY_MCP_TRUST_WORKSPACE`）时，inherit 必须
/// **成功且忽略**未信任配置（仅记诊断），不得升级成 `acp_config_invalid`。
#[test]
fn h2_untrusted_workspace_mcp_config_is_ignored_not_fatal() {
    let _guard: MutexGuard<'static, ()> = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = temp_dir("untrusted-mcp");
    let workspace = root.join("workspace");
    let home = root.join("pony-agent-home");
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::create_dir_all(&home).expect("home");
    std::fs::write(
        workspace.join(".mcp.json"),
        json!({
            "schemaVersion": "mcp-config-v1",
            "mcpServers": {"injected": {"command": "echo", "args": ["should-not-be-used"]}}
        })
        .to_string(),
    )
    .expect("写未信任 .mcp.json");

    let saved_home = std::env::var_os("PONY_AGENT_HOME");
    let saved_trust = std::env::var_os("PONY_MCP_TRUST_WORKSPACE");
    std::env::set_var("PONY_AGENT_HOME", &home);
    std::env::remove_var("PONY_MCP_TRUST_WORKSPACE");

    let mut inherit_spec = spec_with_hint(hint(false, false), "init_capture", &[]);
    inherit_spec.mcp_servers = None; // None ⇒ inherit
    let init_file = workspace.join("initialize.jsonl");
    let session_file = workspace.join("session-new.jsonl");
    inherit_spec.args = vec![
        fixture_script().to_string_lossy().into_owned(),
        "init_capture".to_string(),
        init_file.to_string_lossy().into_owned(),
        "1".to_string(),
        session_file.to_string_lossy().into_owned(),
    ];

    let resolved = resolve_mcp_servers_in(&inherit_spec, &workspace);
    let resolved = match resolved {
        Ok(servers) => servers,
        Err(error) => panic!(
            "h2 违约：未信任 .mcp.json 必须 fail-soft 忽略（诊断记录即可），不得报错；实际 {error:?}"
        ),
    };
    assert!(
        resolved.is_empty(),
        "h2 违约：未信任的 workspace 配置不得进入 wire，实际 {resolved:?}"
    );

    let spawned = AcpAgentSession::spawn(&inherit_spec, &workspace);
    let mut session = match spawned {
        Ok(session) => session,
        Err(error) => panic!(
            "h2 违约：workspace 有未信任 .mcp.json 时 spawn 必须成功（不得 acp_config_invalid），实际 {error:?}"
        ),
    };
    session.close();

    match saved_home {
        Some(value) => std::env::set_var("PONY_AGENT_HOME", value),
        None => std::env::remove_var("PONY_AGENT_HOME"),
    }
    match saved_trust {
        Some(value) => std::env::set_var("PONY_MCP_TRUST_WORKSPACE", value),
        None => std::env::remove_var("PONY_MCP_TRUST_WORKSPACE"),
    }
    let _ = std::fs::remove_dir_all(&root);
}

// ───────────────────────── h3 未声明能力不得被 install 追认 ─────────────────────────

/// h3：caps 全 false 的会话事后 `install_fs_handler` 不得追认能力：
/// agent 的 `fs/read_text_file` / `fs/write_text_file` 必须 JSON-RPC error，不得真实执行。
/// h3b（B177 ADVISORY ⑤ 的 terminal 对偶）：caps 全 false 的会话里，agent 发 `terminal/create`
/// 及后续 `terminal/*` 必须被 JSON-RPC `-32601` + `data.code=acp_unavailable` 拒绝，不得真实执行。
///
/// 与 h3 的差别：h3 走 fs 一侧（`probe_fs`），本条走 terminal 一侧（`probe_terminal` 的 9201..9206），
/// 锁的是分发闸门对**未声明能力**的终态，而不是「装了 handler 会不会追认」。
#[test]
fn h3b_undeclared_terminal_is_rejected_at_dispatch() {
    let cwd = temp_dir("undeclared-terminal");
    let marker = cwd.join("marker.txt");
    std::fs::write(&marker, "TERMINAL-PROBE-MARKER").expect("写 terminal 探针 marker");
    let init_file = cwd.join("initialize.jsonl");
    let resp_file = cwd.join("responses.jsonl");
    let marker_arg = marker.to_string_lossy().into_owned();
    let spec = spec_with_hint(
        hint(false, false), // 未声明任何能力
        "probe_terminal",
        &[
            init_file.to_string_lossy().as_ref(),
            resp_file.to_string_lossy().as_ref(),
            marker_arg.as_str(),
        ],
    );

    let session = match AcpAgentSession::spawn_with_mcp(&spec, &cwd, Vec::new()) {
        Ok(session) => session,
        Err(error) => panic!("h3b：spawn_with_mcp 必须成功：{error:?}"),
    };
    run_prompt(&session);

    let responses = captured_responses(&cwd);
    for id in [9201u64, 9202, 9203, 9204, 9205, 9206] {
        let response = response_for(&responses, id);
        assert!(
            response.get("error").is_some(),
            "h3b 违约：未声明 terminal 的会话必须拒绝 terminal/* 反向请求（id={id}），实际 {response}"
        );
        assert!(
            response.get("result").is_none(),
            "h3b 违约：不得执行未声明的 terminal/* 并返回 result（id={id}），实际 {response}"
        );
        assert_eq!(
            response["error"]["code"].as_i64(),
            Some(-32601),
            "h3b 违约：terminal/* 必须以 JSON-RPC -32601 拒绝（id={id}），实际 {response}"
        );
        assert_eq!(
            response["error"]["data"]["code"].as_str(),
            Some("acp_unavailable"),
            "h3b 违约：错误负载必须带 data.code=acp_unavailable（id={id}），实际 {response}"
        );
        let text = response["error"].to_string();
        assert!(
            !text.contains("TERMINAL-PROBE-MARKER"),
            "h3b 违约：错误消息不得泄漏命令内容（id={id}），实际 {text}"
        );
    }
}

#[test]
fn h3_installed_handler_cannot_enable_undeclared_capability() {
    let cwd = temp_dir("late-handler");
    let target = cwd.join("secret.txt");
    std::fs::write(&target, "TOP-SECRET-CONTENT").expect("写 workspace 文件");
    let target_json = format!("{:?}", target.to_string_lossy().into_owned());
    let init_file = cwd.join("initialize.jsonl");
    let resp_file = cwd.join("responses.jsonl");
    let paths_file = write_paths_file(&cwd, &[target_json.as_str()]);
    let spec = spec_with_hint(
        hint(false, false), // 未声明任何能力
        "probe_fs",
        &[
            init_file.to_string_lossy().as_ref(),
            resp_file.to_string_lossy().as_ref(),
            paths_file.to_string_lossy().as_ref(),
        ],
    );

    let mut session = match AcpAgentSession::spawn_with_mcp(&spec, &cwd, Vec::new()) {
        Ok(session) => session,
        Err(error) => panic!("h3：spawn_with_mcp 必须成功：{error:?}"),
    };
    // 握手后再注入 handler：未声明能力不得因此变为可接受。
    // ADVISORY ⑤：装配返回值必须可观测——被闸门拒绝时返回 `false`（旧签名返回 `()`，静默）。
    assert!(
        !session.install_fs_handler(workspace_fs_handler(&cwd)),
        "h3 违约：握手未声明 fs 时 install_fs_handler 必须返回 false（装配被拒），实际返回 true"
    );
    run_prompt(&session);

    let responses = captured_responses(&cwd);
    for id in [9100u64, 9500u64] {
        let response = response_for(&responses, id);
        assert!(
            response.get("error").is_some(),
            "h3 违约：caps=false 的会话在 install_fs_handler 后仍不得接受 fs 反向请求，实际 {response}"
        );
        assert!(
            response.get("result").is_none(),
            "h3 违约：不得执行未声明能力的 fs/* 并返回 result，实际 {response}"
        );
        // ADVISORY ⑤：不只断言「有 error」，还要锁住错误码与负载形状。
        assert_eq!(
            response["error"]["code"].as_i64(),
            Some(-32601),
            "h3 违约：未声明能力的反向请求必须以 JSON-RPC -32601（method not found）拒绝，实际 {response}"
        );
        assert_eq!(
            response["error"]["data"]["code"].as_str(),
            Some("acp_unavailable"),
            "h3 违约：错误负载必须带 data.code=acp_unavailable（供调用方降级），实际 {response}"
        );
        let text = response["error"].to_string();
        assert!(
            !text.contains("TOP-SECRET-CONTENT"),
            "h3 违约：错误消息不得泄漏文件内容，实际 {text}"
        );
    }
    // 文件内容不得被回传（若实现错误地读了文件，result.content 会泄漏）。
    let combined = responses
        .iter()
        .map(|response| response.to_string())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !combined.contains("TOP-SECRET-CONTENT"),
        "h3 违约：未声明能力的 fs 读取不得真实执行（响应中出现文件内容）：{combined}"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── h4 长命令 wait 不得 5s 硬顶 ─────────────────────────

/// h4：`sh -c 'sleep 6; echo done'` 的 `wait_for_exit` 必须在 NFR 派生预算内正常返回
/// （不得 5s 硬顶成 error），且同轮后续入站请求仍被应答（读线程未饿死）。
#[test]
fn h4_long_running_terminal_wait_respects_budget_and_stays_responsive() {
    let cwd = temp_dir("long-terminal");
    let delay_seconds = 6;
    let init_file = cwd.join("initialize.jsonl");
    let resp_file = cwd.join("responses.jsonl");
    let spec = spec_with_hint(
        hint(false, true),
        "probe_terminal_long",
        &[
            init_file.to_string_lossy().as_ref(),
            resp_file.to_string_lossy().as_ref(),
            &delay_seconds.to_string(),
        ],
    );

    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        None,
        Some(managed_terminal_handler()),
    ) {
        Ok(session) => session,
        Err(error) => panic!("h4：spawn 必须成功：{error:?}"),
    };
    // 整轮预算取 NFR 默认（120s）：避免 prompt 自身 5s 默认预算先撞线，掩盖 wait 的阻断点。
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);
    let elapsed = run_prompt(&session);

    let responses = captured_responses(&cwd);
    let waited = response_for(&responses, 9402);
    assert_eq!(
        waited["result"]["exitCode"],
        json!(0),
        "h4 违约：{}s 长命令的 wait_for_exit 必须正常返回 exitCode=0（不得被 {}ms 硬顶成 error），实际 {waited}",
        delay_seconds,
        nfr_acp_ms("request_timeout_ms")
    );
    // NFR 派生预算：整轮默认预算（120s）内必须完成；且确实跑过了 5s 旧硬顶。
    assert!(
        elapsed > Duration::from_millis(nfr_acp_ms("request_timeout_ms")),
        "h4：本轮应确实超过 {}ms（否则未覆盖旧硬顶边界），实际 {elapsed:?}",
        nfr_acp_ms("request_timeout_ms")
    );
    assert!(
        elapsed < ACP_DEFAULT_TURN_BUDGET,
        "h4 违约：wait 必须在 NFR 派生预算 {ACP_DEFAULT_TURN_BUDGET:?} 内返回，实际 {elapsed:?}"
    );

    // 输出面：wait 之后仍能看到 stdout（与 h1 同源，长命令场景复核）。
    let output = response_for(&responses, 9403);
    assert!(
        output["result"]["output"]
            .as_str()
            .is_some_and(|text| text.contains("done")),
        "h4 违约：wait 之后 output 必须含 `done`，实际 {output}"
    );
    // 非饿死：同轮再发一次无害请求必须被应答（读线程未被 wait 永久占用/崩溃）。
    let follow_up = response_for(&responses, 9404);
    assert!(
        follow_up.get("result").is_some(),
        "h4 违约：wait 之后同轮无害请求必须仍被应答（读线程不得饿死），实际 {follow_up}"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── h5 terminal 输出缓冲有界 ─────────────────────────
//
// 复审 r2 BLOCKING（review-acp-config.json handlers.rs:381）：terminal 输出缓冲
// 在每次 poll 时无界 `push_str` 累积 ⇒ 外部 agent 可持续轮询 output 把宿主内存拖爆。
// 断言只测**缓冲字节上限**（不断言 RSS，CI 不友好）：
// ① 末次 `terminal/output` 的 output 字节数 ≤ `outputByteLimit`（create 显式传 1 MiB/流）；
// ② 协议 `TerminalOutputResponse.truncated` 必须为 true（v1 有该字段；若未来 schema 移除，
//    以 ① 的长度上限作为等价信号）；
// ③ kill/release 仍可用，release + close 后无残留进程。
const H5_OUTPUT_BYTE_LIMIT: usize = 1_048_576;

#[test]
#[cfg_attr(
    windows,
    ignore = "输出洪泛探针依赖 pgrep/POSIX 子进程语义（Windows 无等价命令，B230/B246）"
)]
fn h5_terminal_output_buffer_is_bounded() {
    let cwd = temp_dir("flood-output");
    let marker = format!("pony-h5-{}-{}", std::process::id(), Instant::now().elapsed().as_nanos());
    let marker_file = cwd.join("marker.txt");
    std::fs::write(&marker_file, &marker).expect("写 marker 文件");
    let init_file = cwd.join("initialize.jsonl");
    let resp_file = cwd.join("responses.jsonl");
    let polls = "30";
    let spec = spec_with_hint(
        hint(false, true),
        "probe_terminal_flood",
        &[
            init_file.to_string_lossy().as_ref(),
            resp_file.to_string_lossy().as_ref(),
            marker_file.to_string_lossy().as_ref(),
            polls,
        ],
    );

    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        None,
        Some(managed_terminal_handler()),
    ) {
        Ok(session) => session,
        Err(error) => panic!("h5：spawn 必须成功：{error:?}"),
    };
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);
    run_prompt(&session);

    let responses = captured_responses(&cwd);
    let final_output = response_for(&responses, 9700);
    let text = final_output["result"]["output"]
        .as_str()
        .unwrap_or_else(|| panic!("h5 违约：末次 terminal/output 必须返回 result，实际 {final_output}"));
    assert!(
        text.len() <= H5_OUTPUT_BYTE_LIMIT,
        "h5 违约：terminal 输出缓冲无界累积（{} 次轮询后 output {} 字节 > 上限 {} 字节，\
         外部 agent 可据此 OOM 宿主）",
        polls,
        text.len(),
        H5_OUTPUT_BYTE_LIMIT
    );
    assert_eq!(
        final_output["result"]["truncated"],
        json!(true),
        "h5 违约：达到上限必须显式标记 truncated=true（v1 TerminalOutputResponse 字段），实际 {final_output}"
    );

    for id in [9701u64, 9702u64] {
        let response = response_for(&responses, id);
        assert!(
            response.get("error").is_none(),
            "h5 违约：超限后 kill/release 仍必须可用，实际 {response}"
        );
    }

    session.close();
    // 残留：release + close 后不得再有匹配 marker 的进程（marker 只在终端子进程 argv）。
    let deadline = Duration::from_millis(nfr_acp_ms("spawn_reap_grace_ms") * 4);
    let started = Instant::now();
    loop {
        let status = std::process::Command::new("pgrep")
            .arg("-f")
            .arg(&marker)
            .output()
            .expect("pgrep 可执行");
        if status.status.code() == Some(1) {
            break;
        }
        assert!(
            started.elapsed() < deadline,
            "h5 违约：release+close 后不得残留终端子进程（pgrep exit={:?}），stdout:{}",
            status.status.code(),
            String::from_utf8_lossy(&status.stdout)
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── i1 握手预算（B178-1） ─────────────────────────
//
// [POST-HOC-GREEN: 实现先于用例（B178 已修），仅作回归守护]
// 旧实现把 `initialize` 也套在 transport 单请求 5s 基准上 ⇒ 慢启动 agent 永远握手失败；
// 新实现用 `ACP_HANDSHAKE_TIMEOUT`（30s）。用例注入 >5s、<30s 的 initialize 延迟。
#[test]
fn i1_handshake_budget_exceeds_legacy_five_seconds() {
    let baseline_ms = nfr_acp_ms("request_timeout_ms");
    let delay_ms = baseline_ms + 1000;
    let delay_seconds = format!("{:.3}", delay_ms as f64 / 1000.0);
    let cwd = temp_dir("handshake-budget");
    let spec = spec_with_hint(hint(false, false), "init_delay", &[delay_seconds.as_str()]);

    let started = Instant::now();
    let spawned = AcpAgentSession::spawn(&spec, &cwd);
    let elapsed = started.elapsed();

    let mut session = match spawned {
        Ok(session) => session,
        Err(error) => panic!(
            "i1 违约：initialize 延迟 {delay_ms}ms 在握手预算 {ACP_HANDSHAKE_TIMEOUT:?} 内必须成功（旧 5s 基准会失败），实际 {error:?}"
        ),
    };
    assert!(
        elapsed > Duration::from_millis(baseline_ms),
        "i1：本用例必须真正超过旧 {baseline_ms}ms 基准才有覆盖力，实测 {elapsed:?}"
    );
    assert!(
        elapsed < ACP_HANDSHAKE_TIMEOUT,
        "i1 违约：握手必须在 {ACP_HANDSHAKE_TIMEOUT:?} 预算内完成，实测 {elapsed:?}"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── i2 ACP 子进程 env 扩展（B178-2） ─────────────────────────
//
// [POST-HOC-GREEN: 实现先于用例（B178 已修），仅作回归守护]
// `ACP_CHILD_ENV_EXTRA_ALLOWLIST`（XDG_*/TERM/LANG）必须原样透传；`spec.env` 注入 PATH 必须被
// deny 表拦下（子进程 PATH 仍是宿主 PATH，绝不是注入值）。
#[test]
#[cfg_attr(
    windows,
    ignore = "POSIX 子进程 PATH/allowlist 语义（Git Bash 会重写 PATH，B230/B246）"
)]
fn i2_acp_child_env_extends_allowlist_without_path_injection() {
    let _guard: MutexGuard<'static, ()> = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let cwd = temp_dir("env-extra");
    let out_file = cwd.join("env.txt");
    let unique = format!("pony-i2-{}", std::process::id());
    let expected: Vec<(&'static str, String)> = vec![
        ("XDG_CONFIG_HOME", format!("/tmp/{unique}/config")),
        ("TERM", format!("term-{unique}")),
        ("LANG", "C.UTF-8-i2".to_string()),
    ];
    let saved: Vec<(&'static str, Option<std::ffi::OsString>)> = expected
        .iter()
        .map(|(key, _)| (*key, std::env::var_os(key)))
        .collect();
    let host_path = std::env::var("PATH").unwrap_or_default();
    for (key, value) in &expected {
        std::env::set_var(key, value);
    }

    let mut spec = spec_with_hint(
        hint(false, false),
        "env_capture",
        &[out_file.to_string_lossy().as_ref()],
    );
    // 恶意注入：PATH 属 deny 表，必须被丢弃。
    spec.env = vec![AcpEnvVariable {
        name: "PATH".to_string(),
        value: "/evil-acp-injected".to_string(),
    }];

    let spawn_result = AcpAgentSession::spawn(&spec, &cwd);
    let mut session = match spawn_result {
        Ok(session) => session,
        Err(error) => panic!("i2：spawn 必须成功：{error:?}"),
    };
    session.close();

    for (key, old) in saved {
        match old {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }

    let body = std::fs::read_to_string(&out_file).expect("fake agent 必须写出 env 快照");
    let mut received = std::collections::BTreeMap::new();
    for line in body.lines() {
        if let Some((key, value)) = line.split_once('=') {
            received.insert(key.to_string(), value.to_string());
        }
    }
    for (key, value) in &expected {
        assert_eq!(
            received.get(*key).map(String::as_str),
            Some(value.as_str()),
            "i2 违约：ACP 子进程必须收到宿主 `{key}`（在 ACP_CHILD_ENV_EXTRA_ALLOWLIST 内），实际 {received:?}"
        );
    }
    let child_path = received.get("PATH").cloned().unwrap_or_default();
    assert_ne!(
        child_path, "/evil-acp-injected",
        "i2 违约：PATH 属 deny 表，spec.env 注入必须被丢弃，实际子进程 PATH={child_path:?}"
    );
    assert_eq!(
        child_path, host_path,
        "i2 违约：子进程 PATH 必须是宿主 allowlist 值（不得被注入覆盖），实际 {child_path:?}"
    );

    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── i3 initialize 错误归因（B178-3） ─────────────────────────
//
// [POST-HOC-GREEN: 实现先于用例（B178 已修），仅作回归守护]
// 错误帧必须**如实归因**：认证语义 ⇒ `acp_auth_required`（绝不塌成 acp_timeout）；
// 无认证语义的 -32603 ⇒ `acp_agent_error`（保留原始码/文案）。
#[test]
fn i3_initialize_error_attribution_is_faithful() {
    let cwd = temp_dir("init-error");

    let auth_spec = spec_with_hint(
        hint(false, false),
        "init_error",
        &["-32603", "please authenticate first"],
    );
    let auth_error = match AcpAgentSession::spawn(&auth_spec, &cwd) {
        Ok(mut session) => {
            session.close();
            panic!("i3 违约：agent 回错误帧时 spawn 必须失败，实际成功")
        }
        Err(error) => error,
    };
    assert!(
        matches!(auth_error, AcpTransportError::AuthRequired(_)),
        "i3 违约：认证语义必须归因 AuthRequired，实际 {auth_error:?}"
    );
    assert_eq!(
        auth_error.code(),
        "acp_auth_required",
        "i3 违约：认证错误码必须是 acp_auth_required（不是 acp_timeout），实际 {auth_error:?}"
    );

    let plain_spec = spec_with_hint(hint(false, false), "init_error", &["-32603", "internal boom"]);
    let plain_error = match AcpAgentSession::spawn(&plain_spec, &cwd) {
        Ok(mut session) => {
            session.close();
            panic!("i3 违约：agent 回错误帧时 spawn 必须失败，实际成功")
        }
        Err(error) => error,
    };
    assert!(
        matches!(plain_error, AcpTransportError::Agent(_)),
        "i3 违约：无认证语义的 -32603 必须归因 Agent，实际 {plain_error:?}"
    );
    assert_eq!(
        plain_error.code(),
        "acp_agent_error",
        "i3 违约：非 auth 的 -32603 必须报 acp_agent_error（不得一律当认证错误），实际 {plain_error:?}"
    );

    let _ = std::fs::remove_dir_all(&cwd);
}
