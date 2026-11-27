//! E2E 回归（真实事故 `pa_01a21aa3-533e-7da3-b4af-22774f04699d` 全链路复现）：
//!
//! 1. Turn 1 上游失败（HTTP 500）⇒ 生成 `TurnCancelled` 节点 + trace + 闭合历史；
//! 2. 双 Esc `list_rewind_points` 可列出该失败轮；`rewind_session` 撤回到 Turn 1 之前：
//!    历史清空、原失败节点标 `TurnRewound`、不再可撤回；
//! 3. 继续发起 Turn 3（卡住超过 C5 宽限期）⇒ 用户 `stop_turn` 触发 C5 监督者收口：
//!    监督者在发射 `turn:cancelled` 前即把 Turn 3 闭合历史 + trace 原子写入 SQLite；
//! 4. UI 解锁后立即发起 Turn 4 并成功收口，滞后 Turn 3 响应随后返回：
//!    - Turn 4 上下文包含 Turn 3，绝不包含已被撤回的 Turn 1；
//!    - 滞后 Turn 3 被 terminal-once 闸门静默抑制（无第二条 `turn:completed`、无重复 user/assistant）；
//! 5. 冷重启按 `WriteSeparate` 全量/稀疏重开 SQLite：历史与 trace 完整一致，零复活、零丢失。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::json;

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::control_plane::rewind_commands::{
    RewindMode, RewindPointsQuery, RewindSessionCommand,
};
use pony_agent_core::agent::control_plane::{
    HostControlPlane, HostControlPlaneBuilder, SessionRuntimeViewQuery, StartTurnStreamCommand,
    StopTurnCommand,
};
use pony_agent_core::agent::execution_control::ExecutionControlRegistry;
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntimeBuilder, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::{
    HistoryNodeKind, SeparateTraceTableMode, SessionStore, SNAPSHOT_ROOT_ENV,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;
use pony_agent_core::agent::stop_escalation::{StopEscalation, StopEscalationConfig, StopReason};
use pony_agent_core::agent::turn_flow::TurnEventSink;

const WAIT_BUDGET_MS: u64 = 10_000;
static DIR_COUNTER: AtomicUsize = AtomicUsize::new(1);
static ISOLATE_SANDBOX: Once = Once::new();

fn isolate_sandbox_env() {
    ISOLATE_SANDBOX.call_once(|| {
        let root = std::env::temp_dir().join(format!("pony-pa01a21aa3-it-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        std::env::set_var("XDG_DATA_HOME", &root);
        std::env::set_var("HOME", &root);
        std::env::set_var("LOCALAPPDATA", &root);
        std::env::set_var(SNAPSHOT_ROOT_ENV, root.join("rewind"));
    });
}

fn temp_case_dir(name: &str) -> std::path::PathBuf {
    isolate_sandbox_env();
    let dir = std::env::temp_dir().join(format!(
        "pony-pa01a21aa3-{name}-{}-{}",
        std::process::id(),
        DIR_COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp case dir");
    dir
}

fn wait_until<F: FnMut() -> bool>(mut predicate: F, timeout_ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    while !predicate() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    true
}

#[derive(Default)]
struct RecordingSink {
    captured: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingSink {
    fn events_for_turn(&self, turn_id: &str, name: &str) -> Vec<TurnStreamEvent> {
        self.captured
            .lock()
            .expect("sink lock poisoned")
            .iter()
            .filter(|(n, payload)| n == name && payload.turn_id == turn_id)
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}

impl TurnEventSink for RecordingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.captured
            .lock()
            .expect("sink lock poisoned")
            .push((name.to_string(), payload));
    }
}

enum MockReply {
    Http500(String),
    DelayedJson { delay_ms: u64, text: String },
    ImmediateJson(String),
}

struct MockScenarioServer {
    base_url: String,
    addr: String,
    shutdown: Arc<AtomicBool>,
    requests: Arc<Mutex<Vec<String>>>,
    handle: Option<JoinHandle<()>>,
}

impl MockScenarioServer {
    fn start(replies: Vec<MockReply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let queue = Arc::new(Mutex::new(std::collections::VecDeque::from(replies)));
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let reqs_ref = Arc::clone(&requests);
        let shutdown_ref = Arc::clone(&shutdown);

        let handle = std::thread::spawn(move || {
            while !shutdown_ref.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let body = read_http_request_body(&mut stream);
                if shutdown_ref.load(Ordering::SeqCst) {
                    return;
                }
                reqs_ref.lock().expect("reqs lock").push(body);
                let action = queue.lock().expect("queue lock").pop_front();
                match action {
                    Some(MockReply::Http500(err)) => {
                        let json_err = json!({ "error": { "message": err } }).to_string();
                        let resp = format!(
                            "HTTP/1.1 500 Internal Server Error\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            json_err.len(),
                            json_err
                        );
                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.flush();
                    }
                    Some(MockReply::DelayedJson { delay_ms, text }) => {
                        std::thread::sleep(Duration::from_millis(delay_ms));
                        let json_ok = json!({
                            "choices": [{ "message": { "role": "assistant", "content": text } }],
                            "usage": { "prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10 }
                        })
                        .to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            json_ok.len(),
                            json_ok
                        );
                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.flush();
                    }
                    Some(MockReply::ImmediateJson(text)) => {
                        let json_ok = json!({
                            "choices": [{ "message": { "role": "assistant", "content": text } }],
                            "usage": { "prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10 }
                        })
                        .to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            json_ok.len(),
                            json_ok
                        );
                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.flush();
                    }
                    None => {
                        let json_ok = json!({
                            "choices": [{ "message": { "role": "assistant", "content": "fallback-ok" } }],
                            "usage": { "prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10 }
                        })
                        .to_string();
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            json_ok.len(),
                            json_ok
                        );
                        let _ = stream.write_all(resp.as_bytes());
                        let _ = stream.flush();
                    }
                }
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            shutdown,
            requests,
            handle: Some(handle),
        }
    }

    fn request_bodies(&self) -> Vec<String> {
        self.requests.lock().expect("reqs lock").clone()
    }
}

impl Drop for MockScenarioServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(&self.addr);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_http_request_body(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    let mut header_end: Option<usize> = None;
    let mut content_length = 0usize;
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => buffer.extend_from_slice(&chunk[..read]),
            Err(_) => break,
        }
        if header_end.is_none() {
            if let Some(index) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                header_end = Some(index + 4);
                let headers = String::from_utf8_lossy(&buffer[..index + 4]).to_lowercase();
                for line in headers.lines() {
                    if let Some(value) = line.strip_prefix("content-length:") {
                        content_length = value.trim().parse::<usize>().unwrap_or(0);
                    }
                }
            }
        }
        if let Some(end) = header_end {
            if buffer.len().saturating_sub(end) >= content_length {
                break;
            }
        }
    }
    let body_start = header_end.unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[body_start..]).to_string()
}

struct StaticProviderResolver(ResolvedProviderSelection);

impl ProviderSelectionResolver for StaticProviderResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        self.0.clone()
    }
}

fn provider_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "ctx-e2e".to_string(),
        provider_name: "ctx-e2e".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "CTX_E2E_KEY".to_string(),
        api_key: Some("ctx-e2e-key".to_string()),
        model: "ctx-e2e-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 256,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(8_192),
            supports_tools: false,
            supports_streaming: false,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn turn_input(session_id: &str, message: &str) -> TurnInput {
    TurnInput {
        message: message.to_string(),
        display_message: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some(session_id.to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    }
}

fn build_control_plane(db_path: &Path, workspace: &Path, base_url: String) -> Arc<HostControlPlane> {
    let session_store = SessionStore::with_backend(Box::new(
        SqliteSessionBackend::new_with_trace_mode(
            db_path.to_path_buf(),
            SeparateTraceTableMode::WriteSeparate,
        ),
    ));
    let runtime = AgentRuntimeBuilder::new()
        .session_store(session_store)
        .provider_resolver(Box::new(StaticProviderResolver(provider_selection(base_url))))
        .workspace_root(workspace)
        .build();
    let registry = ExecutionControlRegistry::new();
    let escalation = Arc::new(StopEscalation::with_registry(
        StopEscalationConfig::for_test(40, 10),
        registry.clone(),
    ));
    Arc::new(
        HostControlPlaneBuilder::new()
            .runtime(runtime)
            .execution_control(registry)
            .stop_escalation(escalation)
            .graph_run_store(GraphRunStore::new())
            .build(),
    )
}

#[test]
fn e2e_pa01a21aa3_failed_turn_rewind_c5_cancel_and_restart_context_intact() {
    let _guard = crate::env_lock::lock();
    let dir = temp_case_dir("e2e");
    let db_path = dir.join("sessions.db");
    let workspace = dir.join("ws");
    std::fs::create_dir_all(&workspace).expect("create ws");
    let session_id = "sess-pa01a21aa3-e2e";

    let server = MockScenarioServer::start(vec![
        MockReply::Http500("upstream_boom_turn1".to_string()),
        MockReply::DelayedJson {
            delay_ms: 350,
            text: "LAGGING_TURN3_REPLY".to_string(),
        },
        MockReply::ImmediateJson("TURN4_FINAL_ANSWER".to_string()),
    ]);
    let control_plane = build_control_plane(&db_path, &workspace, server.base_url.clone());
    let sink = Arc::new(RecordingSink::default());

    // 1) Turn 1 失败 ⇒ TurnCancelled 节点落盘 + 可通过双 Esc 撤回。
    let outcome1 = control_plane.start_turn_stream_checked(
        sink.as_ref(),
        StartTurnStreamCommand {
            turn_id: "turn-e2e-1".to_string(),
            input: turn_input(session_id, "FIRST_FAILED_QUESTION"),
        },
    );
    assert!(outcome1.is_ok(), "Turn 1 stream call ok with failed terminal: {outcome1:?}");

    let points = control_plane.list_rewind_points(RewindPointsQuery {
        session_id: Some(session_id.to_string()),
    });
    assert_eq!(points.len(), 1, "失败的 Turn 1 必须出现在撤回菜单中，实际={points:?}");
    assert_eq!(points[0].user_message, "FIRST_FAILED_QUESTION");

    // 2) 双 Esc 撤回 Turn 1。
    let rewound = control_plane
        .rewind_session(RewindSessionCommand {
            session_id: Some(session_id.to_string()),
            node_id: points[0].node_id.clone(),
            mode: RewindMode::ConversationOnly,
        })
        .expect("rewind failed turn 1");
    let view_after_rewind = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
        session_id: Some(session_id.to_string()),
        turn_id: None,
        node_id: None,
        run_id: None,
    });
    assert_eq!(rewound.session_id, session_id);
    assert!(view_after_rewind.session.history.is_empty(), "撤回后历史应清零");
    assert!(
        control_plane
            .list_rewind_points(RewindPointsQuery {
                session_id: Some(session_id.to_string()),
            })
            .is_empty(),
        "撤回后不再有可撤回节点"
    );

    // 3) 发起 Turn 3（provider 延迟 350ms），15ms 后请求 stop_turn 触发 C5 40ms 宽限期收口。
    let cp_for_t3 = Arc::clone(&control_plane);
    let sink_for_t3 = Arc::clone(&sink);
    control_plane.register_turn_event_sink("turn-e2e-3", Arc::clone(&sink));
    let t3_handle = std::thread::spawn(move || {
        cp_for_t3.start_turn_stream(
            sink_for_t3.as_ref(),
            StartTurnStreamCommand {
                turn_id: "turn-e2e-3".to_string(),
                input: turn_input(session_id, "TURN3_INTERRUPTED_QUESTION"),
            },
        );
    });
    assert!(
        wait_until(|| server.request_bodies().len() >= 2, WAIT_BUDGET_MS),
        "Turn 3 must reach mock provider"
    );
    let stop_resp = control_plane.stop_turn(StopTurnCommand {
        turn_id: "turn-e2e-3".to_string(),
        stop_reason: Some(StopReason::User),
    });
    assert!(stop_resp.accepted, "stop_turn must be accepted");

    assert!(
        wait_until(|| !sink.events_for_turn("turn-e2e-3", "turn:cancelled").is_empty(), WAIT_BUDGET_MS),
        "C5 必须及时发出 turn:cancelled"
    );

    // Option A 核心契约：一收到 turn:cancelled，无需等滞后线程退出，SessionStore 已持久化 Turn 3。
    let view_after_c5 = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
        session_id: Some(session_id.to_string()),
        turn_id: None,
        node_id: None,
        run_id: None,
    });
    assert_eq!(
        view_after_c5.session.history.len(),
        2,
        "C5 发射 turn:cancelled 时 Turn 3 必须已原子落盘，实际={:?}",
        view_after_c5.session.history
    );
    assert_eq!(
        view_after_c5.session.history[0].content,
        "TURN3_INTERRUPTED_QUESTION"
    );

    // 4) UI 解锁后立即发起 Turn 4（此时滞后 Turn 3 尚未返回），Turn 4 完成后再等滞后 Turn 3 线程结束。
    let outcome4 = control_plane.start_turn_stream_checked(
        sink.as_ref(),
        StartTurnStreamCommand {
            turn_id: "turn-e2e-4".to_string(),
            input: turn_input(session_id, "TURN4_CONTINUATION_QUESTION"),
        },
    );
    assert!(outcome4.is_ok(), "Turn 4 must finish cleanly: {outcome4:?}");
    t3_handle.join().expect("turn 3 lagging thread joins cleanly");

    let reqs = server.request_bodies();
    let t4_req = reqs.last().expect("Turn 4 HTTP body captured");
    assert!(
        t4_req.contains("TURN3_INTERRUPTED_QUESTION"),
        "Turn 4 上下文必须包含已撤回后重发的 Turn 3；实际请求={t4_req}"
    );
    assert!(
        !t4_req.contains("FIRST_FAILED_QUESTION"),
        "已被撤回的 Turn 1 绝不得泄漏到 Turn 4 上下文；实际请求={t4_req}"
    );

    assert!(
        sink.events_for_turn("turn-e2e-3", "turn:completed").is_empty(),
        "滞后醒来的 Turn 3 不得再发射 turn:completed"
    );
    assert_eq!(
        sink.events_for_turn("turn-e2e-3", "turn:cancelled").len(),
        1,
        "Turn 3 终态事件必须恰好 1 条"
    );

    let live_view = control_plane.load_session_runtime_view(SessionRuntimeViewQuery {
        session_id: Some(session_id.to_string()),
        turn_id: None,
        node_id: None,
        run_id: None,
    });
    assert_eq!(live_view.session.history.len(), 4, "活跃历史只含 Turn 3 与 Turn 4 共 4 条消息");

    // 5) 冷重启重载：全量与稀疏装载均与 live 一致。
    let reloaded = SessionStore::with_backend(Box::new(
        SqliteSessionBackend::new_with_trace_mode(db_path.clone(), SeparateTraceTableMode::WriteSeparate),
    ));
    let snap = reloaded.snapshot_at_readonly(Some(session_id), None, &[]);
    assert_eq!(snap.history.len(), 4);
    assert_eq!(snap.history[0].content, "TURN3_INTERRUPTED_QUESTION");
    assert_eq!(snap.history[2].content, "TURN4_CONTINUATION_QUESTION");
    assert_eq!(snap.history[3].content, "TURN4_FINAL_ANSWER");
    assert!(snap.history_nodes.iter().any(|n| n.kind == HistoryNodeKind::TurnRewound));

    let mut lazy = SessionStore::open_lazy_with_backend(Box::new(
        SqliteSessionBackend::new_with_trace_mode(db_path, SeparateTraceTableMode::WriteSeparate),
    ));
    assert!(lazy.hydrate_session(session_id));
    let lazy_snap = lazy.snapshot_at_readonly(Some(session_id), None, &[]);
    assert_eq!(lazy_snap.history, snap.history);

    let _ = std::fs::remove_dir_all(&dir);
}
