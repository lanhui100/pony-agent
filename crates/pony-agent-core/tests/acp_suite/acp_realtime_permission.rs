//! ACP permission 往返 + update 实时性契约（backlog B161/B162 / task-15）。
//!
//! 真源：`.dev-team/contract-matrix-acp-realtime-permission.md`（P1–P4 / R1–R3 / S1 / N1）。
//!
//! 断言纪律：
//! - permission 往返走**真实** `GovernedDispatcher`（`ToolRegistrySnapshot::builtin` + `FakeClock`），
//!   宿主侧消费走 `approve_control_request` / `cancel_control_request`（真实 CAS 事实）；
//! - 实时性断言带**时间戳证据**：sink 记录 `(Instant, name, payload)`，断言首帧 delta 的 emit 时刻
//!   早于 prompt 返回 ≥ 1s（旧「批量 flush」实现必然失败）；
//! - 阈值取 `.dev-team/nfr-baseline.json` 的 `acp` 段，等待用有界轮询 / `recv_timeout`；
//! - 无 `#[ignore]`、无空断言；只碰 ACP 测试与夹具。

use super::support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use pony_agent_core::acp::config::{AcpAgentSpec, AcpCapabilitiesHint};
use pony_agent_core::acp::protocol::ContentBlock;
use pony_agent_core::acp::session::{AcpAgentSession, DispatcherPermissionHandler};
use pony_agent_core::acp::ACP_DEFAULT_TURN_BUDGET;
use pony_agent_core::agent::dispatcher::{ControlRequestAuthorization, GovernedDispatcher};
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::tool_runtime::{
    FakeClock, PendingControlRequest, PendingControlRequestKind, PendingControlRequestState,
    RuntimeClock,
};
use pony_agent_core::agent::tools::ToolRegistrySnapshot;
use pony_agent_core::agent::turn_flow::TurnEventSink;
use serde_json::Value;

// ───────────────────────────── 设施 ─────────────────────────────

static TEMP_SEQ: AtomicU32 = AtomicU32::new(0);

fn temp_dir(tag: &str) -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "pony-acp-rt-{}-{}-{tag}-{seq}",
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

fn fake_spec(mode: &str, extra: &[&str]) -> AcpAgentSpec {
    let mut args = vec![fixture_script().to_string_lossy().into_owned(), mode.to_string()];
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
        mcp_servers: Some(Vec::new()),
    }
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

fn request_timeout() -> Duration {
    Duration::from_millis(nfr_acp_ms("request_timeout_ms"))
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

/// 带时间戳的事件 sink：`TurnEventSink` 冻结不带 Sync，但测试需要在多处读取 ⇒ Mutex 内部同步。
#[derive(Default)]
struct TimedSink {
    events: Mutex<Vec<(Instant, String, TurnStreamEvent)>>,
}

impl TurnEventSink for TimedSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push((Instant::now(), name.to_string(), payload));
    }
}

impl TimedSink {
    fn snapshot(&self) -> Vec<(Instant, String, TurnStreamEvent)> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

/// `TurnStreamEvent` 未实现 `Debug`：失败信息用序列化形态呈现（证据可读）。
fn payload_summary(payload: &TurnStreamEvent) -> String {
    serde_json::to_string(payload).unwrap_or_else(|error| format!("<unserializable:{error}>"))
}

fn events_summary(events: &[(Instant, String, TurnStreamEvent)]) -> String {
    events
        .iter()
        .map(|(_, name, payload)| format!("{name}:{}", payload_summary(payload)))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn prompt_blocks(text: &str) -> Vec<ContentBlock> {
    vec![ContentBlock::Text {
        text: text.to_string(),
    }]
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

fn captured_permission_answer(cwd: &Path) -> Value {
    let path = cwd.join("permission-answer.jsonl");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("fake agent 必须捕获权限应答（{}：{error}）", path.display()));
    serde_json::from_str(raw.lines().next().expect("权限应答行缺失")).expect("应答必须是合法 JSON")
}

fn answer_outcome(frame: &Value) -> String {
    frame["result"]["outcome"]["outcome"]
        .as_str()
        .map(str::to_string)
        .or_else(|| frame.get("error").map(|error| format!("error:{error}")))
        .unwrap_or_else(|| format!("<unrecognized:{frame}>"))
}

fn permission_prompt_spec(cwd: &Path) -> AcpAgentSpec {
    let capture = cwd.join("permission-answer.jsonl");
    let capture_text = capture.to_string_lossy().into_owned();
    fake_spec("permission_prompt", &[capture_text.as_str(), "acp-sess-1"])
}

fn spawn_with_permission(
    spec: &AcpAgentSpec,
    cwd: &Path,
    permission: Option<Arc<dyn pony_agent_core::acp::client::PermissionHandler>>,
) -> AcpAgentSession {
    match AcpAgentSession::spawn_with_full_options(spec, cwd, Vec::new(), None, None, permission) {
        Ok(session) => session,
        Err(error) => panic!("spawn_with_full_options 必须成功（fake agent 可用）：{error:?}"),
    }
}

// ───────────────────────── P1 正相：approve → selected(optionId∈options) ─────────────────────────

/// P1：prompt 期间 `session/request_permission` 注册为 `PendingControlRequestKind::Approval`；
/// 宿主 approve 后 client 应答 `selected`，且 `optionId` 必须 ∈ request.options。
#[test]
fn p1_host_approval_returns_selected_option() {
    let cwd = temp_dir("permission-approve");
    let dispatcher = real_dispatcher();
    let authorization = ControlRequestAuthorization::default();
    let handler = Arc::new(DispatcherPermissionHandler::new(
        Arc::clone(&dispatcher),
        authorization.clone(),
    ));
    let mut session = spawn_with_permission(&permission_prompt_spec(&cwd), &cwd, Some(handler));
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);

    let sink = TimedSink::default();
    let timeout = request_timeout() * 3;
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = session.prompt(prompt_blocks("needs approval"), &sink);
            let _ = tx.send(result);
        });

        let pending = wait_for_approval(dispatcher.as_ref(), timeout).unwrap_or_else(|| {
            panic!("P1 违约：prompt 期间的 request_permission 必须注册为 Approval 待决请求")
        });
        assert_eq!(pending.request_kind, PendingControlRequestKind::Approval);

        // 宿主批准：CAS 事实从 pending 自身派生（version/nonce/session 同源）。
        let auth = authorization_for(&pending);
        dispatcher
            .approve_control_request(&pending.request_id, &auth)
            .expect("P1：宿主批准必须被真实 dispatcher 接受");

        let result = rx
            .recv_timeout(timeout)
            .expect("P1 违约：宿主批准后 prompt 必须及时返回");
        assert_eq!(result.status, "ok", "P1：prompt 应正常结束，实际 {result:?}");
    });

    let frame = captured_permission_answer(&cwd);
    assert_eq!(
        answer_outcome(&frame),
        "selected",
        "P1 违约：宿主批准后必须应答 selected，实际 {frame}"
    );
    let selected = frame["result"]["outcome"]["optionId"]
        .as_str()
        .unwrap_or_else(|| panic!("P1 违约：selected 必须携带 optionId，实际 {frame}"));
    assert!(
        !selected.is_empty(),
        "P1：optionId 不得为空，实际 {frame}"
    );
    // fake agent 的 options 固定为 allow-once / reject-once ⇒ 断言取值为其中之一（∈ options）。
    assert!(
        selected == "allow-once" || selected == "reject-once",
        "P1 违约：optionId 必须来自 request.options（allow-once/reject-once），实际 {selected}"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── P2 负相：宿主 cancel → cancelled ─────────────────────────

/// P2：宿主 `cancel_control_request` ⇒ client 应答 `{outcome:{outcome:"cancelled"}}`。
#[test]
fn p2_host_cancel_returns_cancelled() {
    let cwd = temp_dir("permission-cancel");
    let dispatcher = real_dispatcher();
    let authorization = ControlRequestAuthorization::default();
    let handler = Arc::new(DispatcherPermissionHandler::new(
        Arc::clone(&dispatcher),
        authorization.clone(),
    ));
    let mut session = spawn_with_permission(&permission_prompt_spec(&cwd), &cwd, Some(handler));
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);

    let sink = TimedSink::default();
    let timeout = request_timeout() * 3;
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = session.prompt(prompt_blocks("needs approval"), &sink);
            let _ = tx.send(result);
        });

        let pending = wait_for_approval(dispatcher.as_ref(), timeout)
            .unwrap_or_else(|| panic!("P2 前置：必须注册 Approval 待决请求"));
        let auth = authorization_for(&pending);
        dispatcher
            .cancel_control_request(&pending.request_id, &auth)
            .expect("P2：宿主取消必须被真实 dispatcher 接受");

        let result = rx
            .recv_timeout(timeout)
            .expect("P2 违约：宿主取消后 prompt 必须及时返回");
        assert_eq!(result.status, "ok", "P2：取消审批不应改变本轮结束语义，实际 {result:?}");
    });

    let frame = captured_permission_answer(&cwd);
    assert_eq!(
        answer_outcome(&frame),
        "cancelled",
        "P2 违约：宿主取消后必须应答 cancelled，实际 {frame}"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── P3 无 permission handler → 一律 cancelled ─────────────────────────

/// P3：`spawn_with_mcp_and_handlers`（permission=None）⇒ 回落 FailClosedPermissionHandler，
/// 一律 cancelled（绝不 selected）。
#[test]
fn p3_missing_permission_handler_is_always_cancelled() {
    let cwd = temp_dir("permission-none");
    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &permission_prompt_spec(&cwd),
        &cwd,
        Vec::new(),
        None,
        None,
    ) {
        Ok(session) => session,
        Err(error) => panic!("P3：spawn 必须成功：{error:?}"),
    };
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);

    let sink = TimedSink::default();
    let result = session.prompt(prompt_blocks("needs approval"), &sink);
    assert_eq!(result.status, "ok", "P3：fail-closed 应答后本轮应正常结束，实际 {result:?}");

    let frame = captured_permission_answer(&cwd);
    let outcome = answer_outcome(&frame);
    assert_eq!(
        outcome, "cancelled",
        "P3 违约：无 permission handler 时必须 cancelled，实际 {outcome} / {frame}"
    );
    assert_ne!(outcome, "selected", "P3 硬约束：绝不默认 allow");

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── P4 有 handler 但宿主不响应 → cancelled（有界） ─────────────────────────

/// P4：有 permission handler 但宿主始终不响应超过 `acp.request_timeout_ms` ⇒ cancelled，
/// 且 prompt 不永久挂住（耗时 ≤ 预算 + NFR 派生 margin）。
#[test]
fn p4_unanswered_permission_becomes_cancelled_within_budget() {
    let cwd = temp_dir("permission-timeout");
    let dispatcher = real_dispatcher();
    let handler = Arc::new(DispatcherPermissionHandler::new(
        Arc::clone(&dispatcher),
        ControlRequestAuthorization::default(),
    ));
    let mut session = spawn_with_permission(&permission_prompt_spec(&cwd), &cwd, Some(handler));
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);

    let sink = TimedSink::default();
    let started = Instant::now();
    let result = session.prompt(prompt_blocks("needs approval"), &sink);
    let elapsed = started.elapsed();

    assert_eq!(result.status, "ok", "P4：超时应答后本轮应正常结束，实际 {result:?}");
    let frame = captured_permission_answer(&cwd);
    assert_eq!(
        answer_outcome(&frame),
        "cancelled",
        "P4 违约：宿主无响应必须 cancelled（绝不 allow），实际 {frame}"
    );
    let budget = request_timeout();
    let margin = Duration::from_millis(nfr_acp_ms("spawn_reap_grace_ms") * 2);
    assert!(
        elapsed <= budget + margin,
        "P4 违约：等待必须有界（预算 {budget:?} + margin {margin:?}），实际 {elapsed:?}"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── R1 实时性时序 ─────────────────────────

/// R1：fake agent 先发一条 delta 再 sleep 2s 才 end_turn ⇒ sink 必须在 prompt **返回之前**
/// 收到该 delta（旧「批量 flush」实现会拖到返回后，差异 ~0）。
#[test]
fn r1_first_delta_is_emitted_before_prompt_returns() {
    let cwd = temp_dir("realtime");
    let delay_seconds = "2.000";
    let spec = fake_spec("slow_prompt", &[delay_seconds, "acp-sess-1"]);
    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        None,
        None,
    ) {
        Ok(session) => session,
        Err(error) => panic!("R1：spawn 必须成功：{error:?}"),
    };
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);

    let sink = TimedSink::default();
    let result = session.prompt(prompt_blocks("realtime"), &sink);
    let returned_at = Instant::now();
    assert_eq!(result.status, "ok", "R1：本轮应正常结束，实际 {result:?}");

    let events = sink.snapshot();
    let (first_delta_at, _, first_delta) = events
        .iter()
        .find(|(_, name, _)| name == "turn:delta")
        .unwrap_or_else(|| {
            panic!("R1 违约：sink 必须收到 turn:delta，实际 {}", events_summary(&events))
        });
    assert_eq!(
        first_delta.text.as_deref(),
        Some("hello"),
        "R1：首帧 delta 载荷应为 agent_message_chunk(text=\"hello\")，实际 {}",
        payload_summary(first_delta)
    );
    let lead = returned_at.saturating_duration_since(*first_delta_at);
    assert!(
        lead >= Duration::from_secs(1),
        "R1 违约：delta 必须在 prompt 返回**之前**实时 emit（旧实现批量 flush 时 lead≈0），实际间隔 {lead:?}"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── R2 顺序 + R3 不丢不重 ─────────────────────────

/// R2/R3：多条 update 的 emit 顺序 == agent 发送顺序；每帧恰好一次（含最后一条）。
#[test]
fn r2_r3_updates_are_ordered_and_emitted_once() {
    let cwd = temp_dir("ordering");
    let spec = fake_spec("updates", &["-", "acp-sess-1"]);
    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        None,
        None,
    ) {
        Ok(session) => session,
        Err(error) => panic!("R2：spawn 必须成功：{error:?}"),
    };
    session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);

    let sink = TimedSink::default();
    let result = session.prompt(prompt_blocks("ordering"), &sink);
    assert_eq!(result.status, "ok", "R2：本轮应正常结束，实际 {result:?}");

    let events = sink.snapshot();
    // 期望序列（fake `updates` 模式，§3.2.7 映射表）：
    // - agent_message_chunk → turn:delta(text=Some("hello"), reasoning_content=None)
    // - agent_thought_chunk → turn:delta(text=None, reasoning_content=Some("reasoning"))
    // ⇒ 正文 delta 与思考 delta 都是 `turn:delta`，故文本序列必须是 [Some("hello"), None]；
    // 逐条比对（含顺序），并分别校验 text / reasoning_content 两侧字段。
    let deltas: Vec<(Option<String>, Option<String>)> = events
        .iter()
        .filter(|(_, name, _)| name == "turn:delta")
        .map(|(_, _, payload)| (payload.text.clone(), payload.reasoning_content.clone()))
        .collect();
    assert_eq!(
        deltas.len(),
        2,
        "R2 违约：message_chunk 与 thought_chunk 必须各映射一条 turn:delta，实际 {deltas:?}"
    );
    assert_eq!(
        deltas[0].0.as_deref(),
        Some("hello"),
        "R2 违约：第 1 条正文 delta 的 text 必须为 hello，实际 {deltas:?}"
    );
    assert_eq!(
        deltas[0].1, None,
        "R2 违约：正文 delta 不得携带 reasoning_content，实际 {deltas:?}"
    );
    assert_eq!(
        deltas[1].0, None,
        "R2 违约：§3.2.7 的 agent_thought_chunk 映射为 turn:delta(text=None)，实际 {deltas:?}"
    );
    assert_eq!(
        deltas[1].1.as_deref(),
        Some("reasoning"),
        "R2 违约：第 2 条 thought delta 必须带 reasoning_content=reasoning，实际 {deltas:?}"
    );
    assert_eq!(
        deltas,
        vec![
            (Some("hello".to_string()), None),
            (None, Some("reasoning".to_string())),
        ],
        "R2 违约：delta 序列必须与 agent 发送顺序逐条一致（不得只做集合包含）"
    );
    let tool_events: Vec<&(Instant, String, TurnStreamEvent)> = events
        .iter()
        .filter(|(_, name, _)| name == "turn:tool")
        .collect();
    assert_eq!(
        tool_events.len(),
        1,
        "R3 违约：tool_call 必须恰好 emit 一次，实际 {}",
        tool_events.len()
    );
    assert!(
        tool_events[0]
            .2
            .tool_activities
            .as_ref()
            .is_some_and(|activities| activities.iter().any(|activity| activity.id.contains("tc-1")
                || activity.name.contains("tc-1")
                || activity.description.contains("a.rs"))),
        "R2：tool 事件必须指向 tc-1，实际 {}",
        events_summary(&events)
    );
    // 顺序：delta(hello) < delta(reasoning) < tool(tc-1)
    let index_of = |predicate: &dyn Fn(&TurnStreamEvent) -> bool| {
        events
            .iter()
            .position(|(_, name, payload)| {
                (name == "turn:delta" || name == "turn:tool") && predicate(payload)
            })
            .unwrap_or(usize::MAX)
    };
    let hello_index = index_of(&|payload| payload.text.as_deref() == Some("hello"));
    let reasoning_index = index_of(&|payload| payload.reasoning_content.as_deref() == Some("reasoning"));
    let tool_index = index_of(&|payload| payload.tool_activities.is_some());
    assert!(
        hello_index < reasoning_index && reasoning_index < tool_index,
        "R2 违约：emit 顺序必须与 agent 发送顺序一致（hello < reasoning < tool），实际 {hello_index}/{reasoning_index}/{tool_index}"
    );

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── S1 取消仍生效 ─────────────────────────

/// S1：`set_cancellation_token` + prompt 期间取消 ⇒ aborted/tool_cancelled 且 < 整轮预算。
#[test]
fn s1_cancellation_still_aborts_within_budget() {
    use pony_agent_core::agent::budget::CancellationToken;

    let cwd = temp_dir("cancel-realtime");
    let marker = cwd.join("prompt-received.marker");
    let marker_text = marker.to_string_lossy().into_owned();
    let spec = fake_spec("cancel_hold", &[marker_text.as_str(), "acp-sess-1"]);
    let mut session = match AcpAgentSession::spawn_with_mcp_and_handlers(
        &spec,
        &cwd,
        Vec::new(),
        None,
        None,
    ) {
        Ok(session) => session,
        Err(error) => panic!("S1：spawn 必须成功：{error:?}"),
    };
    let budget = Duration::from_millis(nfr_acp_ms("request_timeout_ms") * 4);
    session.set_turn_budget(budget);
    let token = Arc::new(CancellationToken::new());
    session.set_cancellation_token(Arc::clone(&token));

    let sink = TimedSink::default();
    let (tx, rx) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let result = session.prompt(prompt_blocks("long running"), &sink);
            let _ = tx.send(result);
        });
        assert!(
            wait_until(|| marker.exists(), request_timeout()),
            "S1 前置：fake agent 必须已收到 session/prompt"
        );
        token.cancel();
        let result = rx
            .recv_timeout(budget * 2)
            .expect("S1 违约：取消后 prompt 必须及时返回");
        assert_eq!(result.status, "aborted", "S1 违约：取消必须归约 aborted，实际 {result:?}");
        assert!(
            result.output.contains("tool_cancelled"),
            "S1 违约：取消错误码必须是 tool_cancelled，实际 {}",
            result.output
        );
    });

    session.close();
    let _ = std::fs::remove_dir_all(&cwd);
}

// ───────────────────────── N1 既有入口等价（回归锚） ─────────────────────────

/// N1：`spawn` / `spawn_with_mcp` / `spawn_with_mcp_and_handlers` 行为等价：
/// handler 全 None ⇒ 不声明能力、permission 恒 cancelled。
#[test]
fn n1_legacy_entries_keep_fail_closed_permission() {
    let cwd = temp_dir("legacy-equivalence");
    for entry in ["spawn_with_mcp", "spawn_with_mcp_and_handlers"] {
        let capture = cwd.join(format!("permission-answer-{entry}.jsonl"));
        let capture_text = capture.to_string_lossy().into_owned();
        let spec = fake_spec(
            "permission_prompt",
            &[capture_text.as_str(), "acp-sess-1"],
        );
        let mut session = match entry {
            "spawn_with_mcp" => match AcpAgentSession::spawn_with_mcp(&spec, &cwd, Vec::new()) {
                Ok(session) => session,
                Err(error) => panic!("N1：{entry} 必须成功：{error:?}"),
            },
            _ => match AcpAgentSession::spawn_with_mcp_and_handlers(
                &spec,
                &cwd,
                Vec::new(),
                None,
                None,
            ) {
                Ok(session) => session,
                Err(error) => panic!("N1：{entry} 必须成功：{error:?}"),
            },
        };
        session.set_turn_budget(ACP_DEFAULT_TURN_BUDGET);
        let sink = TimedSink::default();
        let result = session.prompt(prompt_blocks("legacy"), &sink);
        assert_eq!(result.status, "ok", "N1：{entry} 本轮应正常结束，实际 {result:?}");

        let raw = std::fs::read_to_string(&capture)
            .unwrap_or_else(|error| panic!("N1：{entry} 必须捕获权限应答（{error}）"));
        let frame: Value = serde_json::from_str(raw.lines().next().expect("应答行缺失"))
            .expect("应答必须是合法 JSON");
        assert_eq!(
            answer_outcome(&frame),
            "cancelled",
            "N1 回归违约：{entry} 的 permission 必须 cancelled（绝不 allow），实际 {frame}"
        );
        session.close();
    }
    let _ = std::fs::remove_dir_all(&cwd);
}
