//! ACP P1 会话驱动 + acp_delegate 工具面验收契约（B150-P1-T，冻结于 task-4）。
//!
//! 真源：`docs/architecture/acp-client-integration.md` §3.2（尤其 §3.2.3/§3.2.6/§3.2.7/§3.2.8）、
//! §4 P1 表；契约矩阵：`.dev-team/contract-matrix-acp-p1.md`（P1-1 ~ P1-8）。
//!
//! 纪律：
//! - 全部走**真实链路**：真实 ACP 子进程（`tests/fixtures/acp_fake_agent.sh`）+ 真实
//!   `GovernedDispatcher`（不伪造 dispatcher、不绕过治理链）；阈值唯一真源是
//!   `.dev-team/nfr-baseline.json` 的 `acp` 段。
//! - 并发等待一律「有界条件轮询 / `recv_timeout`」，不用 `sleep` 当同步。
//! - 红相语义：`acp::client` / `acp::session` / `execute_acp_delegate` 仍是 `unimplemented!()`
//!   时必须真实 panic；不得用 `#[ignore]` 或空断言掩盖。
//!
//! 关于 P1-4 的 Ask 往返口径（Lead 2026-11-14 裁决）：
//! `resolve_permission` 必须把 `session/request_permission` 注册成
//! `PendingControlRequestKind::Approval`，由宿主经 `GovernedDispatcher` 的
//! `approve_control_request` / `cancel_control_request` 消费；本文件不臆造 answer JSON
//! schema（只需 `selected.optionId ∈ request.options`）。

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use pony_agent_core::acp::client::AcpAgentClient;
use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::protocol::{
    ContentBlock, RequestPermissionOutcome, RequestPermissionRequest, StopReason,
};
use pony_agent_core::acp::session::{
    AcpAgentSession, AcpTurnOutcome, ACP_ERROR_AGENT_REFUSED, ACP_TOOL_CANCELLED,
};
use pony_agent_core::acp::transport::AcpTransportError;
use pony_agent_core::agent::dispatcher::{ControlRequestAuthorization, GovernedDispatcher};
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::tool_runtime::{
    FakeClock, PendingControlRequest, PendingControlRequestKind, PendingControlRequestState,
    RuntimeClock,
};
use pony_agent_core::agent::tools::{
    builtin_tools, builtin_turn_tool_contract_views, canonical_tool_name, primitive_dispatch_table,
    ToolResult, ToolRegistrySnapshot,
};
use pony_agent_core::agent::turn_flow::TurnEventSink;
use serde_json::{json, Value};

// ───────────────────────────── 通用设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-p1-{}-{}-{tag}-{seq}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    std::fs::create_dir_all(&path).expect("创建临时工作目录");
    path.canonicalize().expect("canonicalize 临时工作目录")
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_dir_all(path);
}

/// NFR 唯一真源：`.dev-team/nfr-baseline.json`（禁在本文件自选阈值）。
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

/// 本文件所有阻塞等待的上界：NFR 的 acp.request_timeout_ms。
fn op_timeout() -> Duration {
    Duration::from_millis(nfr_acp_ms("request_timeout_ms"))
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

/// 有界条件轮询（到达条件即返回；超时才算失败）。
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

fn prompt_blocks(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
}

/// 真实治理链 dispatcher（与 `tests/ask_pending_contract.rs` 同一套真实对象，非 mock）。
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

fn spawn_session(mode: &str, extra: &[&str], cwd: &Path) -> AcpAgentSession {
    let spec = fake_spec(mode, extra);
    match AcpAgentSession::spawn(&spec, cwd) {
        Ok(session) => session,
        Err(error) => panic!("AcpAgentSession::spawn 失败（fake agent 应可用）: {error:?}"),
    }
}

#[derive(Default)]
struct CapturingSink {
    events: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl TurnEventSink for CapturingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((name.to_string(), payload));
    }
}

impl CapturingSink {
    fn snapshot(&self) -> Vec<(String, TurnStreamEvent)> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// `TurnStreamEvent` 未实现 `Debug`：失败信息用其序列化形态呈现（证据可读）。
fn events_summary(events: &[(String, TurnStreamEvent)]) -> String {
    events
        .iter()
        .map(|(name, payload)| {
            format!(
                "{name}:{}",
                serde_json::to_string(payload).unwrap_or_else(|error| format!("<unserializable:{error}>"))
            )
        })
        .collect::<Vec<_>>()
        .join(" | ")
}

fn permission_request() -> RequestPermissionRequest {
    serde_json::from_value(json!({
        "sessionId": "acp-sess-1",
        "toolCall": {
            "toolCallId": "tc-perm-1",
            "title": "edit a.rs",
            "kind": "edit",
            "status": "pending"
        },
        "options": [
            {"optionId": "allow-once", "name": "Allow once", "kind": "allow_once"},
            {"optionId": "reject-once", "name": "Reject once", "kind": "reject_once"}
        ]
    }))
    .expect("权限请求夹具必须符合 v1 schema")
}

/// 在 dispatcher 里等一条 `PendingControlRequestKind::Approval` 出现（有界条件轮询）。
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

/// 从 pending request 派生 CAS 授权事实（version/nonce/session 必须逐字段同源）。
fn authorization_for(pending: &PendingControlRequest) -> ControlRequestAuthorization {
    ControlRequestAuthorization {
        session_id: pending.session_id.clone(),
        expected_version: pending.version,
        nonce: pending.nonce.clone(),
        ..Default::default()
    }
}

/// 在 JSON 树里找键名含 `needle` 的布尔值（用于 truncated 等聚合标志，不锁死嵌套形状）。
fn find_bool_flag(value: &Value, needle: &str) -> Option<bool> {
    match value {
        Value::Object(map) => map.iter().find_map(|(key, child)| {
            if key.to_lowercase().contains(needle) {
                child.as_bool()
            } else {
                find_bool_flag(child, needle)
            }
        }),
        Value::Array(items) => items.iter().find_map(|child| find_bool_flag(child, needle)),
        _ => None,
    }
}

// ───────────────────────── P1-1 protocolVersion 校验（fail-closed） ─────────────────────────

/// P1-1：initialize 必须以整数 `1` 声明协议版本；agent 回 `2` ⇒ `acp_protocol_error`（不降级）。
#[test]
fn p1_01_initialize_version_mismatch_fails_closed() {
    let cwd = temp_dir("version-check");
    let capture = cwd.join("initialize-request.jsonl");
    let capture_text = capture.to_string_lossy().into_owned();
    let spec = fake_spec("init_capture", &[capture_text.as_str(), "2"]);

    let result = AcpAgentClient::connect(&spec, &cwd);
    let error = match result {
        Ok(mut client) => {
            client.close();
            panic!("P1-1 违约：agent 回 protocolVersion=2 时 connect 必须 fail-closed");
        }
        Err(error) => error,
    };
    assert!(
        matches!(error, AcpTransportError::Protocol(_)),
        "P1-1 违约：版本不匹配必须归约为 Protocol，实际 {error:?}"
    );
    assert_eq!(
        error.code(),
        "acp_protocol_error",
        "P1-1 违约：错误码必须是 acp_protocol_error"
    );

    let raw = std::fs::read_to_string(&capture).expect("fake agent 必须记录 initialize 原始请求");
    let frame: Value = serde_json::from_str(raw.lines().next().expect("initialize 行缺失"))
        .expect("initialize 报文必须是合法 JSON");
    assert_eq!(frame["method"], json!("initialize"));
    assert_eq!(
        frame["params"]["protocolVersion"],
        json!(1),
        "P1-1 违约：客户端必须声明整数 1（不是字符串/0.1）"
    );
    assert!(
        frame["params"]["protocolVersion"].is_number(),
        "P1-1 违约：protocolVersion 必须是 JSON number，实际 {}",
        frame["params"]["protocolVersion"]
    );
    assert!(
        frame["params"].get("clientCapabilities").is_some(),
        "P1-1：initialize 必须携带 clientCapabilities（能力按 spec 声明，fail-closed）"
    );

    cleanup(&cwd);
}

// ───────────────────────── P1-2 绝对 cwd ─────────────────────────

/// P1-2：`session/new` 的 cwd 必须是绝对路径；相对路径在 connect 与 new_session 两处都 fail-closed。
#[test]
fn p1_02_relative_cwd_rejected() {
    let cwd = temp_dir("absolute-cwd");
    let spec = fake_spec("flow", &["end_turn", "acp-sess-1"]);
    let relative = Path::new("relative-workspace");

    // (1) connect 阶段的相对 cwd 必须被拒（transport 层绝对路径校验）。
    match AcpAgentClient::connect(&spec, relative) {
        Ok(mut client) => {
            client.close();
            panic!("P1-2 违约：相对 cwd 不得 connect 成功");
        }
        Err(_) => {}
    }

    // (2) 已握手后 session/new 传相对 cwd 仍必须被拒（plumbing 不得静默 canonicalize）。
    let mut client = match AcpAgentClient::connect(&spec, &cwd) {
        Ok(client) => client,
        Err(error) => panic!("绝对 cwd 必须可连接，实际 {error:?}"),
    };
    let rejected = client.new_session(relative, Vec::new());
    assert!(
        rejected.is_err(),
        "P1-2 违约：session/new 相对 cwd 必须被拒，实际成功"
    );

    // (3) 绝对 cwd 正常建会话，sessionId 来自 agent。
    let response = match client.new_session(&cwd, Vec::new()) {
        Ok(response) => response,
        Err(error) => panic!("绝对 cwd 的 session/new 必须成功，实际 {error:?}"),
    };
    assert_eq!(response.session_id, "acp-sess-1", "P1-2：sessionId 必须原样返回");
    client.close();
    cleanup(&cwd);
}

// ───────────────────────── P1-3 update → TurnEventSink 映射 ─────────────────────────

/// P1-3：`agent_message_chunk`/`agent_thought_chunk`/`tool_call` 分别映射为
/// `turn:delta`(text) / `turn:delta`(reasoning_content) / `turn:tool`。
#[test]
fn p1_03_session_update_maps_to_turn_events() {
    let cwd = temp_dir("update-mapping");
    let mut session = spawn_session("updates", &["-", "acp-sess-1"], &cwd);
    let sink = CapturingSink::default();

    let result = session.prompt(prompt_blocks("do the thing"), &sink);
    assert_eq!(result.status, "ok", "P1-3 前置：end_turn 轮应为成功，实际 {result:?}");

    let events = sink.snapshot();
    assert!(
        events
            .iter()
            .any(|(name, payload)| name == "turn:delta" && payload.text.as_deref() == Some("hello")),
        "P1-3 违约：agent_message_chunk 必须映射为 turn:delta(text=\"hello\")，实际 {}", events_summary(&events)
    );
    assert!(
        events.iter().any(|(name, payload)| name == "turn:delta"
            && payload.reasoning_content.as_deref() == Some("reasoning")),
        "P1-3 违约：agent_thought_chunk 必须映射为 turn:delta(reasoning_content)，实际 {}", events_summary(&events)
    );
    assert!(
        events.iter().any(|(name, payload)| {
            name == "turn:tool"
                && payload
                    .tool_activities
                    .as_ref()
                    .is_some_and(|activities| {
                        activities.iter().any(|activity| {
                            activity.id.contains("tc-1")
                                || activity.name.contains("tc-1")
                                || activity.description.contains("a.rs")
                        })
                    })
        }),
        "P1-3 违约：tool_call 必须映射为 turn:tool(TurnToolActivity)，实际 {}", events_summary(&events)
    );

    session.close();
    cleanup(&cwd);
}

// ───────────────────────── P1-4 权限：fail-closed + Ask 往返 ─────────────────────────

/// P1-4（负半）：无宿主审批可用时，`session/request_permission` 必须回 `cancelled` 或错误，
/// **绝不** 回 `selected`（治理链默认拒绝，§6 风险第一条）。
#[test]
fn p1_04_permission_without_host_is_fail_closed() {
    let cwd = temp_dir("permission-fail-closed");
    let capture = cwd.join("permission-answer.jsonl");
    let capture_text = capture.to_string_lossy().into_owned();
    let mut session = spawn_session(
        "permission_prompt",
        &[capture_text.as_str(), "acp-sess-1"],
        &cwd,
    );
    let sink = CapturingSink::default();

    let result = session.prompt(prompt_blocks("edit the file"), &sink);
    assert_eq!(result.status, "ok", "P1-4 前置：fake agent 以 end_turn 收尾，实际 {result:?}");

    let raw = std::fs::read_to_string(&capture)
        .expect("fake agent 必须记录 client 对 session/request_permission 的应答");
    let line = raw.lines().next().expect("权限应答行缺失");
    let frame: Value = serde_json::from_str(line).expect("权限应答必须是合法 JSON");
    let recorded = frame["result"]["outcome"]["outcome"]
        .as_str()
        .map(str::to_string)
        .or_else(|| frame.get("error").map(|error| format!("error:{error}")))
        .unwrap_or_else(|| format!("<unrecognized:{line}>"));
    assert!(
        recorded == "cancelled" || recorded.starts_with("error"),
        "P1-4 违约：无宿主审批时必须回 cancelled 或 JSON-RPC error，实际 {recorded}"
    );
    assert_ne!(
        recorded, "selected",
        "P1-4 硬约束：无宿主/未决时绝不能默认 allow（selected）"
    );

    session.close();
    cleanup(&cwd);
}

/// P1-4（正半）：`resolve_permission` 必须注册 `PendingControlRequestKind::Approval`，
/// 宿主经真实 dispatcher 的 `approve_control_request` 消费后返回 `selected`（optionId ∈ options）。
#[test]
fn p1_04_permission_ask_roundtrip_returns_selected() {
    let cwd = temp_dir("permission-ask");
    let mut session = spawn_session("flow", &["end_turn", "acp-sess-1"], &cwd);
    let dispatcher = real_dispatcher();
    let request = permission_request();
    let authorization = ControlRequestAuthorization::default();
    let timeout = op_timeout();

    let (sender, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let outcome = session.resolve_permission(&request, dispatcher.as_ref(), &authorization);
            let _ = sender.send(outcome);
        });

        let pending = wait_for_approval(dispatcher.as_ref(), timeout).unwrap_or_else(|| {
            panic!(
                "P1-4 违约：resolve_permission 必须注册 PendingControlRequestKind::Approval，实际无待决审批"
            )
        });
        assert_eq!(pending.request_kind, PendingControlRequestKind::Approval);
        assert_eq!(
            pending.session_id.as_deref(),
            Some("acp-sess-1"),
            "P1-4：审批必须绑定 ACP 会话 id"
        );

        let approval_auth = authorization_for(&pending);
        dispatcher
            .approve_control_request(&pending.request_id, &approval_auth)
            .expect("宿主批准必须被 dispatcher 接受（CAS 事实同源）");

        let response = receiver
            .recv_timeout(timeout)
            .expect("P1-4 违约：宿主批准后 resolve_permission 必须及时返回")
            .unwrap_or_else(|error| panic!("P1-4：resolve_permission 返回错误: {error}"));
        match response.outcome {
            RequestPermissionOutcome::Selected { option_id } => {
                let allowed: Vec<&str> = request
                    .options
                    .iter()
                    .map(|option| option.option_id.as_str())
                    .collect();
                assert!(
                    allowed.contains(&option_id.as_str()),
                    "P1-4 违约：selected.optionId 必须来自请求的 options，实际 {option_id} / {allowed:?}"
                );
            }
            RequestPermissionOutcome::Cancelled => {
                panic!("P1-4 违约：宿主已批准，仍返回 cancelled（默认拒绝不得吞掉批准）")
            }
        }

        let leaked = dispatcher.pending_request(&pending.request_id);
        assert!(
            leaked
                .as_ref()
                .map(|request| request.state != PendingControlRequestState::Pending)
                .unwrap_or(true),
            "P1-4：批准后不得残留 Pending 状态审批，实际 {leaked:?}"
        );
    });

    session.close();
    cleanup(&cwd);
}

// ───────────────────────── P1-5 取消链路 ─────────────────────────

/// P1-5：`session.cancel()` 必须让挂起的 `request_permission` 收到 `cancelled`（不悬挂、不泄漏）。
#[test]
fn p1_05_cancel_settles_pending_permission() {
    let cwd = temp_dir("cancel-permission");
    let mut session = spawn_session("flow", &["end_turn", "acp-sess-1"], &cwd);
    let dispatcher = real_dispatcher();
    let request = permission_request();
    let authorization = ControlRequestAuthorization::default();
    let timeout = op_timeout();

    let (sender, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let outcome = session.resolve_permission(&request, dispatcher.as_ref(), &authorization);
            let _ = sender.send(outcome);
        });

        let pending = wait_for_approval(dispatcher.as_ref(), timeout).unwrap_or_else(|| {
            panic!("P1-5 前置：resolve_permission 必须先注册 Approval 待决请求")
        });

        session
            .cancel()
            .expect("P1-5：cancel 必须能发送 session/cancel notification");

        let response = receiver
            .recv_timeout(timeout)
            .expect("P1-5 违约：cancel 后挂起的 resolve_permission 必须返回 cancelled，实际悬挂")
            .unwrap_or_else(|error| panic!("P1-5：resolve_permission 返回错误: {error}"));
        assert_eq!(
            response.outcome,
            RequestPermissionOutcome::Cancelled,
            "P1-5 违约：session/cancel 后挂起权限请求必须收到 cancelled"
        );

        let leaked = dispatcher.pending_request(&pending.request_id);
        assert!(
            leaked
                .as_ref()
                .map(|request| request.state != PendingControlRequestState::Pending)
                .unwrap_or(true),
            "P1-5 违约：取消后不得残留 Pending 审批，实际 {leaked:?}"
        );
    });

    session.close();
    cleanup(&cwd);
}

/// P1-5（prompt 侧）：cancel 后正在跑的 prompt 必须以 `aborted` + `tool_cancelled` 归约返回。
#[test]
fn p1_05_cancel_aborts_running_prompt() {
    let cwd = temp_dir("cancel-prompt");
    let marker = cwd.join("prompt-received.marker");
    let marker_text = marker.to_string_lossy().into_owned();
    let mut session = spawn_session("cancel_hold", &[marker_text.as_str(), "acp-sess-1"], &cwd);
    let sink = CapturingSink::default();
    let timeout = op_timeout();

    let (sender, receiver) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = session.prompt(prompt_blocks("long running task"), &sink);
            let _ = sender.send(result);
        });

        assert!(
            wait_until(|| marker.exists(), timeout),
            "P1-5 前置：fake agent 必须已收到 session/prompt"
        );
        session
            .cancel()
            .expect("P1-5：运行中 prompt 必须可被 cancel");

        let result: ToolResult = receiver
            .recv_timeout(timeout)
            .expect("P1-5 违约：cancel 后 prompt 必须及时返回，实际悬挂");
        assert_eq!(
            result.status, "aborted",
            "P1-5 违约：cancel 归约状态必须是 aborted，实际 {result:?}"
        );
        assert!(
            result.output.contains(ACP_TOOL_CANCELLED),
            "P1-5 违约：cancel 归约错误码必须是 {ACP_TOOL_CANCELLED}，实际 {}",
            result.output
        );
    });

    session.close();
    cleanup(&cwd);
}

// ───────────────────────── P1-6 五个 stopReason 归约 ─────────────────────────

/// P1-6：`reduce_stop_reason` 纯函数口径（5 → 4 类）。
#[test]
fn p1_06_reduce_stop_reason_pure_mapping() {
    assert_eq!(
        AcpAgentSession::reduce_stop_reason(StopReason::EndTurn),
        AcpTurnOutcome::Ok
    );
    assert_eq!(
        AcpAgentSession::reduce_stop_reason(StopReason::MaxTokens),
        AcpTurnOutcome::Truncated
    );
    assert_eq!(
        AcpAgentSession::reduce_stop_reason(StopReason::MaxTurnRequests),
        AcpTurnOutcome::Truncated
    );
    assert_eq!(
        AcpAgentSession::reduce_stop_reason(StopReason::Refusal),
        AcpTurnOutcome::Refused
    );
    assert_eq!(
        AcpAgentSession::reduce_stop_reason(StopReason::Cancelled),
        AcpTurnOutcome::Cancelled
    );
}

/// P1-6（端到端）：五个 stopReason 经 prompt 归约为 ToolResult 状态 / 错误码。
#[test]
fn p1_06_prompt_reduces_five_stop_reasons() {
    let cwd = temp_dir("stop-reasons");

    // end_turn → ok
    {
        let mut session = spawn_session("flow", &["end_turn", "acp-sess-1"], &cwd);
        let sink = CapturingSink::default();
        let result = session.prompt(prompt_blocks("x"), &sink);
        assert_eq!(result.status, "ok", "P1-6：end_turn 必须归约为 ok，实际 {result:?}");
        session.close();
    }

    // max_tokens / max_turn_requests → ok + truncated:true
    for reason in ["max_tokens", "max_turn_requests"] {
        let mut session = spawn_session("flow", &[reason, "acp-sess-1"], &cwd);
        let sink = CapturingSink::default();
        let result = session.prompt(prompt_blocks("x"), &sink);
        assert_eq!(
            result.status, "ok",
            "P1-6：{reason} 必须仍是成功（ok），实际 {result:?}"
        );
        let output: Value = serde_json::from_str(&result.output)
            .unwrap_or_else(|error| panic!("{reason} 输出必须是 JSON: {error}\n{}", result.output));
        assert_eq!(
            find_bool_flag(&output, "truncat"),
            Some(true),
            "P1-6 违约：{reason} 必须标记 truncated=true，实际 {}",
            result.output
        );
        session.close();
    }

    // refusal → error + acp_agent_refused
    {
        let mut session = spawn_session("flow", &["refusal", "acp-sess-1"], &cwd);
        let sink = CapturingSink::default();
        let result = session.prompt(prompt_blocks("x"), &sink);
        assert_eq!(
            result.status, "error",
            "P1-6：refusal 必须归约为 error，实际 {result:?}"
        );
        assert!(
            result.output.contains(ACP_ERROR_AGENT_REFUSED),
            "P1-6 违约：refusal 错误码必须是 {ACP_ERROR_AGENT_REFUSED}，实际 {}",
            result.output
        );
        session.close();
    }

    // cancelled → aborted + tool_cancelled
    {
        let mut session = spawn_session("flow", &["cancelled", "acp-sess-1"], &cwd);
        let sink = CapturingSink::default();
        let result = session.prompt(prompt_blocks("x"), &sink);
        assert_eq!(
            result.status, "aborted",
            "P1-6：cancelled 必须归约为 aborted，实际 {result:?}"
        );
        assert!(
            result.output.contains(ACP_TOOL_CANCELLED),
            "P1-6 违约：cancelled 错误码必须是 {ACP_TOOL_CANCELLED}，实际 {}",
            result.output
        );
        session.close();
    }

    cleanup(&cwd);
}

// ───────────────────────── P1-7 崩溃 → 错误而非 panic ─────────────────────────

/// P1-7：子进程崩溃必须归约为 `acp_protocol_error` / `acp_unavailable`（含 spawn 失败），非 panic。
#[test]
fn p1_07_crashed_agent_is_error_not_panic() {
    let cwd = temp_dir("crash");
    let spec = fake_spec("crash", &[]);
    let timeout = op_timeout();
    let grace = Duration::from_millis(nfr_acp_ms("spawn_reap_grace_ms"));

    let started = Instant::now();
    let result = AcpAgentClient::connect(&spec, &cwd);
    let error = match result {
        Ok(mut client) => {
            client.close();
            panic!("P1-7 违约：已崩溃的 agent 不得 connect 成功");
        }
        Err(error) => error,
    };
    assert!(
        matches!(
            error,
            AcpTransportError::Spawn(_)
                | AcpTransportError::Protocol(_)
                | AcpTransportError::Unavailable(_)
        ),
        "P1-7 违约：崩溃必须归约为传输层错误，实际 {error:?}"
    );
    assert!(
        started.elapsed() < timeout + grace * 4,
        "P1-7：崩溃必须及时暴露，实际耗时 {:?}",
        started.elapsed()
    );
    cleanup(&cwd);
}

// ───────────────────────── P1-8 acp_delegate 工具面 ─────────────────────────

/// P1-8：`acp_delegate` 的 schema / 命名归一 / 派发表臂 / 契约视图四项齐全。
#[test]
fn p1_08_acp_delegate_tool_surface() {
    let definitions = builtin_tools();
    let definition = definitions
        .iter()
        .find(|definition| definition.name == "acp_delegate")
        .unwrap_or_else(|| {
            panic!(
                "P1-8 违约：builtin_tools() 必须包含 acp_delegate，实际 {} 个工具",
                definitions.len()
            )
        });

    let schema = &definition.input_schema;
    assert_eq!(schema["type"], json!("object"), "P1-8：schema 必须是 object");
    let properties = schema["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("P1-8 违约：acp_delegate 必须有 properties，实际 {schema}"));
    let mut keys: Vec<&str> = properties.keys().map(String::as_str).collect();
    keys.sort_unstable();
    // 仓库全局惯例：`with_description()` 给每个内置工具注入必填的 `description` 参数
    // （schema.rs:23-50，模型侧 UI 摘要用），因此它属于**结构性**参数而非业务参数。
    assert_eq!(
        keys,
        vec!["agent", "cwd", "description", "prompt", "timeoutMs"],
        "P1-8 违约：业务参数面必须是 agent/prompt/cwd?/timeoutMs?（+ 全局 description），实际 {schema}"
    );
    let required: Vec<&str> = schema["required"]
        .as_array()
        .unwrap_or_else(|| panic!("P1-8 违约：required 必须是数组，实际 {schema}"))
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut required_sorted = required.clone();
    required_sorted.sort_unstable();
    assert_eq!(
        required_sorted,
        vec!["agent", "description", "prompt"],
        "P1-8 违约：required 必须恰为 agent + prompt（+ 全局 description），实际 {required:?}"
    );
    assert!(
        !required.contains(&"cwd") && !required.contains(&"timeoutMs"),
        "P1-8 违约：cwd/timeoutMs 必须可选，实际 required={required:?}"
    );
    assert_eq!(
        schema["additionalProperties"],
        json!(false),
        "P1-8 违约：acp_delegate 必须 additionalProperties:false"
    );
    assert_eq!(properties["agent"]["type"], json!("string"));
    assert_eq!(properties["prompt"]["type"], json!("string"));
    assert_eq!(properties["cwd"]["type"], json!("string"));
    assert_eq!(properties["timeoutMs"]["type"], json!("integer"));

    for spelling in ["acp_delegate", "AcpDelegate", "acp.delegate"] {
        assert_eq!(
            canonical_tool_name(spelling),
            Some("acp_delegate"),
            "P1-8 违约：canonical_tool_name 必须把 {spelling} 归一到 acp_delegate"
        );
    }

    assert!(
        primitive_dispatch_table().contains(&"acp_delegate"),
        "P1-8 违约：PRIMITIVE_DISPATCH_TABLE 必须含 acp_delegate 臂，实际 {:?}",
        primitive_dispatch_table()
    );

    let views = builtin_turn_tool_contract_views();
    let view = views
        .iter()
        .find(|view| view.execution_primitive == "acp_delegate")
        .unwrap_or_else(|| {
            panic!(
                "P1-8 违约：contract view 工具面必须包含 acp_delegate，实际 {} 项",
                views.len()
            )
        });
    assert_eq!(view.canonical_tool_name, "acp_delegate");
    assert_eq!(
        view.exposure, "model_visible",
        "P1-8 违约：acp_delegate 必须模型可见，实际 {}",
        view.exposure
    );
    assert_eq!(
        view.kind, "composite",
        "P1-8 违约：acp_delegate 必须是复合工具，实际 {}",
        view.kind
    );

    // 派发路由可执行性：真实 registry 的 dispatch 表必须承认该 primitive（缺臂会走 unsupported）。
    let dispatcher = real_dispatcher();
    assert!(
        dispatcher
            .registry()
            .model_name_for_primitive("acp_delegate")
            .is_some(),
        "P1-8 违约：真实 registry 必须能解析 acp_delegate 的模型面名称"
    );
}
