//! B161 生产路径 permission 注入端到端（j1，POST-HOC 回归）。
//!
//! 契约（task-17）：`DispatchContext.acp_permission: Option<Arc<dyn PermissionHandler>>`
//! → `ToolExecutionContext.acp_permission` → `execute_acp_delegate` →
//! `AcpAgentSession::spawn_with_full_options(.., permission)`。
//!
//! 本文件**只走真实工具路径**（`ToolRouter::execute_with_context` 调 `acp_delegate`），
//! 不直接调 session 方法；宿主侧消费走真实 `GovernedDispatcher`
//! （`approve_control_request` / `cancel_control_request`）。
//!
//! [POST-HOC-GREEN: 生产接线（task-17）先于用例，仅作回归守护]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pony_agent_core::acp::client::PermissionHandler;
use pony_agent_core::acp::session::DispatcherPermissionHandler;
use pony_agent_core::agent::dispatcher::{
    ControlRequestAuthorization, DispatchContext, GovernedDispatcher,
};
use pony_agent_core::agent::tool_runtime::{
    FakeClock, PendingControlRequest, PendingControlRequestKind, PendingControlRequestState,
    RuntimeClock,
};
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolRegistrySnapshot, ToolRouter};
use serde_json::{json, Value};

// ───────────────────────────── 设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-tool-perm-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    path.canonicalize().expect("canonicalize 临时工作目录")
}

fn fixture_script() -> String {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("acp_fake_agent.sh");
    assert!(script.is_file(), "fake agent fixture 缺失: {}", script.display());
    script.to_string_lossy().into_owned()
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

fn wait_until<F: FnMut() -> bool>(mut condition: F, timeout: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    condition()
}

fn real_dispatcher() -> Arc<GovernedDispatcher> {
    let registry = Arc::new(
        ToolRegistrySnapshot::builtin().expect("builtin tool registry 必须构造成功"),
    );
    let clock = Arc::new(FakeClock::new(1_000));
    Arc::new(GovernedDispatcher::new(
        registry,
        Arc::clone(&clock) as Arc<dyn RuntimeClock>,
    ))
}

fn wait_for_approval(
    dispatcher: &GovernedDispatcher,
    timeout: Duration,
) -> Option<PendingControlRequest> {
    let mut found: Option<PendingControlRequest> = None;
    wait_until(
        || {
            found = dispatcher.pending_requests().into_iter().find(|request| {
                request.request_kind == PendingControlRequestKind::Approval
                    && request.state == PendingControlRequestState::Pending
            });
            found.is_some()
        },
        timeout,
    );
    found
}

fn authorization_for(pending: &PendingControlRequest) -> ControlRequestAuthorization {
    ControlRequestAuthorization {
        session_id: pending.session_id.clone(),
        expected_version: pending.version,
        nonce: pending.nonce.clone(),
        ..Default::default()
    }
}

/// 隔离 XDG 配置目录（`acp_delegate` 经 `resolve_agent` 读这里）。
fn install_agents_config(config_home: &Path, agents: Value) {
    let dir = config_home.join("pony-agent");
    std::fs::create_dir_all(&dir).expect("创建配置目录");
    std::fs::write(
        dir.join("acp-agents.json"),
        serde_json::to_string_pretty(&agents).expect("清单可序列化"),
    )
    .expect("写入 acp-agents.json");
}

fn captured_answer(path: &Path) -> Value {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("fake agent 必须捕获权限应答（{}：{error}）", path.display()));
    serde_json::from_str(raw.lines().next().expect("权限应答行缺失")).expect("应答必须是合法 JSON")
}

/// 用真实工具路径调 `acp_delegate`（handler 经 `DispatchContext`/`ToolExecutionContext` 注入）。
/// 返回 `(tool_result_status, 捕获的权限应答帧)`。
fn run_acp_delegate_with_host(
    root: &Path,
    agent_id: &str,
    capture_path: &Path,
    host_action: HostAction,
) -> (String, Value) {
    let workspace = root.join("workspace");
    std::fs::create_dir_all(&workspace).expect("创建 workspace");
    let dispatcher = real_dispatcher();
    let authorization = ControlRequestAuthorization::default();
    let handler: Arc<dyn PermissionHandler> = Arc::new(DispatcherPermissionHandler::new(
        Arc::clone(&dispatcher),
        authorization.clone(),
    ));

    let router = ToolRouter::with_workspace_root(workspace.clone());
    let call = ToolCall {
        call_id: Some(format!("call-{agent_id}")),
        name: "acp_delegate".to_string(),
        arguments: json!({
            "agent": agent_id,
            "prompt": "needs approval",
            "timeoutMs": 30_000,
            "description": "走真实工具路径委派（permission 端到端）"
        }),
        plan: None,
    };
    let context = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some("session-e2e".to_string()),
        dispatch_context: Some(DispatchContext {
            session_id: Some("session-e2e".to_string()),
            turn_id: Some("turn-e2e".to_string()),
            acp_permission: Some(Arc::clone(&handler)),
            ..Default::default()
        }),
        // task-17 冻结：工具上下文直接透传同一 handler 句柄。
        acp_permission: Some(handler),
        ..Default::default()
    };

    let timeout = Duration::from_millis(nfr_acp_ms("request_timeout_ms") * 6);
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = router.execute_with_context(&call, &context);
            let _ = tx.send(result);
        });

        let pending = wait_for_approval(dispatcher.as_ref(), timeout).unwrap_or_else(|| {
            panic!("j1 违约：工具路径的 prompt 期间必须注册 PendingControlRequestKind::Approval")
        });
        let auth = authorization_for(&pending);
        let _ = match host_action {
            HostAction::Approve => dispatcher
                .approve_control_request(&pending.request_id, &auth)
                .expect("j1：宿主批准必须被真实 dispatcher 接受"),
            HostAction::Cancel => dispatcher
                .cancel_control_request(&pending.request_id, &auth)
                .expect("j1：宿主取消必须被真实 dispatcher 接受"),
        };

        let result = rx
            .recv_timeout(timeout)
            .expect("j1 违约：宿主决定后 acp_delegate 必须及时返回");
        let frame = captured_answer(capture_path);
        (result.status, frame)
    })
}

#[derive(Clone, Copy)]
enum HostAction {
    Approve,
    Cancel,
}

// ───────────────────────── j1 生产路径 permission ─────────────────────────

#[test]
fn j1_tool_path_permission_roundtrip() {
    let _guard: MutexGuard<'static, ()> = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let root = temp_dir("tool-permission");
    let config_home = root.join("config");
    let approve_capture = root.join("approve-answer.jsonl");
    let cancel_capture = root.join("cancel-answer.jsonl");
    install_agents_config(
        &config_home,
        json!([
            {
                "id": "fake-approve",
                "label": "Fake approve",
                "command": "sh",
                "args": [fixture_script(), "permission_prompt", approve_capture.to_string_lossy(), "acp-sess-1"]
            },
            {
                "id": "fake-cancel",
                "label": "Fake cancel",
                "command": "sh",
                "args": [fixture_script(), "permission_prompt", cancel_capture.to_string_lossy(), "acp-sess-1"]
            }
        ]),
    );

    let saved_xdg = std::env::var_os("XDG_CONFIG_HOME");
    std::env::set_var("XDG_CONFIG_HOME", &config_home);

    // ① 宿主批准 ⇒ agent 收到 selected(optionId ∈ options)
    let (status, approved_frame) = run_acp_delegate_with_host(
        &root,
        "fake-approve",
        &approve_capture,
        HostAction::Approve,
    );
    assert_eq!(status, "ok", "j1：批准路径 acp_delegate 必须成功返回");
    let outcome = approved_frame["result"]["outcome"]["outcome"]
        .as_str()
        .unwrap_or_else(|| panic!("j1 违约：agent 必须收到 selected 应答，实际 {approved_frame}"));
    assert_eq!(outcome, "selected", "j1 违约：宿主批准后必须是 selected");
    let option_id = approved_frame["result"]["outcome"]["optionId"]
        .as_str()
        .unwrap_or_else(|| panic!("j1 违约：selected 必须带 optionId，实际 {approved_frame}"));
    assert!(
        option_id == "allow-once" || option_id == "reject-once",
        "j1 违约：optionId 必须 ∈ request.options（allow-once/reject-once），实际 {option_id}"
    );

    // ② 宿主取消 ⇒ agent 收到 cancelled
    let (cancel_status, cancelled_frame) = run_acp_delegate_with_host(
        &root,
        "fake-cancel",
        &cancel_capture,
        HostAction::Cancel,
    );
    assert_eq!(cancel_status, "ok", "j1：取消路径 acp_delegate 必须成功返回");
    let cancel_outcome = cancelled_frame["result"]["outcome"]["outcome"]
        .as_str()
        .unwrap_or_else(|| panic!("j1 违约：agent 必须收到 cancelled 应答，实际 {cancelled_frame}"));
    assert_eq!(
        cancel_outcome, "cancelled",
        "j1 违约：宿主取消后必须是 cancelled（不得 selected）"
    );

    match saved_xdg {
        Some(value) => std::env::set_var("XDG_CONFIG_HOME", value),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    let _ = std::fs::remove_dir_all(&root);
}
