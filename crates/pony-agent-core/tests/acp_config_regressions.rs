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

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pony_agent_core::acp::config::{resolve_mcp_servers_in, AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::handlers::{ManagedTerminalHandler, WorkspaceFsHandler};
use pony_agent_core::acp::protocol::ContentBlock;
use pony_agent_core::acp::session::AcpAgentSession;
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
    path.canonicalize().expect("canonicalize 临时工作目录")
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
    session.install_fs_handler(workspace_fs_handler(&cwd));
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
