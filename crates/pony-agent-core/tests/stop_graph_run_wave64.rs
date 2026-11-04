//! wave-64 黑盒验收（独立测试者，红相先写）：桌面停止即时生效 + 用户取消不得归类为错误。
//!
//! 契约真值源：`.dev-team/contracts/wave-64-stop-terminal-status.md` §2 R1 / R2。
//!
//! 覆盖：
//! - R1：对**存在活跃 turn** 的 graph run 调 `stop_graph_run` ⇒ 同步派发**恰好一条**
//!   `turn:stop_requested`（`stopReason="user"`、`kind="stop_requested"`）；宽限监督（测试注入
//!   毫秒级小值，禁 sleep 2s）在宽限期到点后幂等收口并派发**恰好一条** `turn:cancelled`
//!   （`stopReason="user"`）；同一 run/turn 重复 stop 不重复派发终态。
//! - R1 负例：run 不存在 ⇒ 保持既有错误返回，且**不派发任何事件**。
//! - R2：工具执行期（工具返回 `aborted`）用户取消 ⇒ 终态 `turn:cancelled` 而非 `turn:failed`；
//!   provider 读通道以 `provider_stream_cancelled` 结束且用户取消 ⇒ 同样归 `turn:cancelled`。
//!
//! 纪律（test-expert L2）：真实断言、AAA、命名带判据号；禁止 `#[ignore]` / catch 吞异常 /
//! 空断言；所有跨线程等待都是**显式条件轮询**（`wait_until`）+ 硬上界，绝不裸 sleep 盲等。
//!
//! R2 的可达性说明（契约要求"以能真实驱动的最小夹具为准"）：
//! - 桌面停止的真实路径是 `stop_graph_run` → `ExecutionControlRegistry::request_stop`，
//!   而"取消 vs 失败"的分流发生在 `AgentRuntime` 的流式轮词表里，故 R2 走
//!   `AgentRuntime::start_turn_stream_with_control`（真实流式轮入口，public）+ 本地回环
//!   mock provider（同步 JSON / SSE 停帧两种形态）+ 注入式 `ToolExecutor`（返回 `aborted`，
//!   并在执行中发出真实的 stop 请求，等价于用户在工具执行期点停止）。
//!   不使用任何 crate 内部私有入口，也不 mock 被断言的分流逻辑本身。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::context::DefaultTurnContextBuilder;
use pony_agent_core::agent::control_plane::{
    ExecutionCheckpointQuery, HostControlPlane, HostControlPlaneBuilder, StartGraphRunStreamCommand,
    StopGraphRunCommand, StopTurnCommand,
};
use pony_agent_core::agent::execution_control::ExecutionControlRegistry;
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::planner::LocalTurnPlanner;
use pony_agent_core::agent::runtime::{
    AgentRuntime, AgentRuntimeBuilder, TurnInput, TurnStreamEvent,
};
use pony_agent_core::agent::session::SessionStore;
use pony_agent_core::agent::stop_escalation::{StopEscalation, StopEscalationConfig, StopReason};
use pony_agent_core::agent::telemetry::DefaultTurnTelemetryBuilder;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionStatus, ToolExecutor, ToolResult};
use pony_agent_core::agent::turn_flow::TurnEventSink;

/// 宽限监督注入值（毫秒级，禁 sleep 2s 的真实宽限期）。
const TEST_GRACE_MS: u64 = 60;
const TEST_POLL_TICK_MS: u64 = 5;
/// 显式条件轮询的硬上界（契约 NFR watchdog 5s 以内）。
const WAIT_BUDGET_MS: u64 = 5_000;

/// 集成测试（`tests/*.rs`）编译 crate 时不启用 `cfg(test)`，`SessionStore::new()`
/// 会走生产分支指向用户真实 `~/.local/share/PonyAgent/sessions.db`。这里在**任何**
/// 控制面/运行时构造之前（`Once` 保证顺序）把数据目录基准重定向到本测试二进制独有的
/// 临时沙箱，确保集成测试绝不读写生产数据（与 graph_run_lock_concurrency_red.rs 同法）。
static ISOLATE_PROCESS_DATA_DIR: Once = Once::new();

fn isolate_process_data_dir() {
    ISOLATE_PROCESS_DATA_DIR.call_once(|| {
        let unique = format!(
            "pony-agent-it-wave64-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        );
        let sandbox = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&sandbox).expect("create isolated data dir");
        std::env::set_var("XDG_DATA_HOME", &sandbox);
        std::env::set_var("HOME", &sandbox);
        std::env::set_var("LOCALAPPDATA", &sandbox);
    });
}

fn isolated_session_store() -> SessionStore {
    isolate_process_data_dir();
    SessionStore::new()
}

/// 记录型 sink：捕获 `(name, payload)`，用于断言事件条数与载荷字段。
#[derive(Default)]
struct RecordingSink {
    captured: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self::default()
    }

    fn events_named(&self, name: &str) -> Vec<TurnStreamEvent> {
        self.captured
            .lock()
            .expect("recording sink lock poisoned")
            .iter()
            .filter(|(captured_name, _)| captured_name == name)
            .map(|(_, payload)| payload.clone())
            .collect()
    }

    fn names(&self) -> Vec<String> {
        self.captured
            .lock()
            .expect("recording sink lock poisoned")
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }
}

impl TurnEventSink for RecordingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.captured
            .lock()
            .expect("recording sink lock poisoned")
            .push((name.to_string(), payload));
    }
}

/// 显式条件轮询（禁止裸 sleep 盲等）：谓词为真即返回 true，超时返回 false。
fn wait_until<F: FnMut() -> bool>(mut predicate: F, timeout_ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn test_turn_input(session_id: &str, message: &str) -> TurnInput {
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

/// 内存会话 + 内存 graph run store 的控制面（绝不触碰用户真实数据目录）。
fn control_plane_with_test_escalation(
    registry: ExecutionControlRegistry,
) -> HostControlPlane {
    let runtime = AgentRuntimeBuilder::new()
        .session_store(isolated_session_store())
        .build();
    HostControlPlaneBuilder::new()
        .runtime(runtime)
        .graph_run_store(GraphRunStore::new())
        .execution_control(registry.clone())
        .stop_escalation(Arc::new(StopEscalation::with_registry(
            StopEscalationConfig::for_test(TEST_GRACE_MS, TEST_POLL_TICK_MS),
            registry,
        )))
        .build()
}

/// 造一个**存在活跃 turn** 的 graph run（prepare 阶段结束即 Running + active_turn_id，
/// 不执行任何真实 provider 调用），返回 `(run_id, turn_id)`。
fn prepare_graph_run_with_active_turn(
    control_plane: &HostControlPlane,
    run_id: &str,
    turn_id: &str,
    session_id: &str,
) {
    control_plane
        .prepare_start_graph_run_stream(StartGraphRunStreamCommand {
            turn_id: turn_id.to_string(),
            run_id: Some(run_id.to_string()),
            goal: "wave-64 acceptance: stop must be immediate and idempotent".to_string(),
            input: test_turn_input(session_id, "执行一个会被用户停止的轮次"),
        })
        .expect("wave-64 夹具：graph run 必须能被 prepare 到 Running + 活跃 turn");
}

// ---------------------------------------------------------------------------
// R1：桌面停止路径享有 C2/C5 语义
// ---------------------------------------------------------------------------

/// R1：`stop_graph_run` 必须同步派发恰好一条 `turn:stop_requested`，
/// 并由宽限监督幂等收口到恰好一条 `turn:cancelled`（`stopReason="user"`）。
#[test]
fn r1_stop_graph_run_dispatches_single_stop_requested_and_grace_cancels_turn() {
    // Arrange
    let registry = ExecutionControlRegistry::new();
    let control_plane = control_plane_with_test_escalation(registry.clone());
    let sink = Arc::new(RecordingSink::new());
    let run_id = "run-wave64-r1";
    let turn_id = "run-wave64-r1-turn-1";
    let session_id = "session-wave64-r1";

    prepare_graph_run_with_active_turn(&control_plane, run_id, turn_id, session_id);
    control_plane.register_turn_event_sink(turn_id, Arc::clone(&sink));

    // Act
    let response = control_plane
        .stop_graph_run(StopGraphRunCommand {
            run_id: run_id.to_string(),
        })
        .expect("R1：存在活跃 turn 的 graph run 必须可停止");

    // Assert ①：同步派发恰好一条 turn:stop_requested（stopReason=user）
    let stop_requested = sink.events_named("turn:stop_requested");
    assert_eq!(
        stop_requested.len(),
        1,
        "R1：stop_graph_run 被接受时必须**同步**派发恰好一条 turn:stop_requested，实际事件序列 = {:?}",
        sink.names()
    );
    let stop_event = &stop_requested[0];
    assert_eq!(stop_event.kind, "stop_requested", "R1：事件 kind 必须是 stop_requested");
    assert_eq!(stop_event.turn_id, turn_id, "R1：事件必须指向活跃 turn");
    assert_eq!(
        stop_event.stop_reason.as_deref(),
        Some("user"),
        "R1：turn:stop_requested 必须带 stopReason=\"user\""
    );
    assert_eq!(
        response.turn_stop.as_ref().map(|stop| stop.stop_reason.clone()),
        Some(Some("user".to_string())),
        "R1：控制面应答必须回带 turn 停止原因 user"
    );

    // Assert ②：宽限监督到点收口（注入 60ms，禁 sleep 2s）⇒ 恰好一条 turn:cancelled
    let cancelled_arrived = wait_until(|| sink.events_named("turn:cancelled").len() >= 1, WAIT_BUDGET_MS);
    assert!(
        cancelled_arrived,
        "R1：宽限期（注入 {TEST_GRACE_MS}ms）内循环未退出时，监督者必须在 {WAIT_BUDGET_MS}ms 内收口并派发 turn:cancelled；实际事件序列 = {:?}",
        sink.names()
    );
    // 收口**幂等**：等到终态后再给监督者一个极短观察窗口，仍不得出现第二条终态。
    std::thread::sleep(Duration::from_millis(TEST_GRACE_MS * 2));
    let cancelled = sink.events_named("turn:cancelled");
    assert_eq!(
        cancelled.len(),
        1,
        "R1：宽限监督必须**幂等**收口 —— turn:cancelled 恰好一条，实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        cancelled[0].stop_reason.as_deref(),
        Some("user"),
        "R1：turn:cancelled 必须带 stopReason=\"user\""
    );

    // Assert ③：终态落盘为 cancelled，且再次 stop 不重复派发
    let checkpoint = control_plane
        .load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: Some(turn_id.to_string()),
            session_id: None,
        })
        .expect("R1：活跃 turn 必须有 checkpoint");
    assert_eq!(checkpoint.status, "cancelled", "R1：宽限收口后 checkpoint 终态必须是 cancelled");
    assert_eq!(
        checkpoint.stop_reason.as_deref(),
        Some("user"),
        "R1：checkpoint 必须保留 stop_reason=user"
    );

    // 重复 stop：不论既有实现返回 Ok/Err，事件都不得重复派发（幂等判据）。
    let _repeat = control_plane.stop_graph_run(StopGraphRunCommand {
        run_id: run_id.to_string(),
    });
    assert_eq!(
        sink.events_named("turn:stop_requested").len(),
        1,
        "R1：重复 stop 不得重复派发 turn:stop_requested，实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        sink.events_named("turn:cancelled").len(),
        1,
        "R1：重复 stop 不得重复派发终态，实际事件序列 = {:?}",
        sink.names()
    );
}

/// R1 负例：run 不存在 ⇒ 既有错误返回，且不派发任何事件。
#[test]
fn r1_stop_graph_run_unknown_run_keeps_error_and_emits_nothing() {
    // Arrange
    let registry = ExecutionControlRegistry::new();
    let control_plane = control_plane_with_test_escalation(registry);
    let sink = Arc::new(RecordingSink::new());
    let run_id = "run-wave64-r1-known";
    let turn_id = "run-wave64-r1-known-turn-1";
    prepare_graph_run_with_active_turn(&control_plane, run_id, turn_id, "session-wave64-r1b");
    control_plane.register_turn_event_sink(turn_id, Arc::clone(&sink));

    // Act
    let result = control_plane.stop_graph_run(StopGraphRunCommand {
        run_id: "run-wave64-does-not-exist".to_string(),
    });

    // Assert
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("R1：不存在的 run 必须保持既有错误返回，实际返回了成功应答"),
    };
    assert!(
        error.contains("not found"),
        "R1：错误信息必须如实说明 run 不存在，实际 = {error}"
    );
    assert!(
        sink.names().is_empty(),
        "R1：run 不存在时不得派发任何事件，实际事件序列 = {:?}",
        sink.names()
    );
    let checkpoint = control_plane
        .load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: Some(turn_id.to_string()),
            session_id: None,
        })
        .expect("R1：活跃 turn 必须有 checkpoint");
    assert!(
        checkpoint.stop_requested_at_ms.is_none(),
        "R1：run 不存在时不得为无关 turn 置停止位"
    );
}

// ---------------------------------------------------------------------------
// R2：用户取消不得被归类为错误（真实流式轮 + 本地回环 provider）
// ---------------------------------------------------------------------------

/// mock provider 的应答形态。
enum MockReply {
    /// 同步 JSON 决策：返回一次工具调用（`stream: false` 路径）。
    JsonToolCall,
    /// SSE 响应：发完响应头后**不再发任何帧**（停帧），用于驱动 provider 读通道的取消码。
    SseStall,
}

struct MockProviderServer {
    base_url: String,
    addr: String,
    requests: Arc<Mutex<Vec<String>>>,
    shutdown: Arc<AtomicBool>,
    /// 上游**已收到**请求的确定性信号（用例以它作为「in-band 循环已进入 provider 读」的握手点）。
    seen: Arc<AtomicBool>,
    /// SSE 停帧的**释放闸门**：置 true 后 mock 关闭连接，in-band 循环随即恢复并收口。
    release: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockProviderServer {
    fn start(reply: MockReply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock provider 必须能绑定回环端口");
        let port = listener.local_addr().expect("mock provider 地址").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let requests_for_thread = Arc::clone(&requests);
        let shutdown_for_thread = Arc::clone(&shutdown);
        let seen_for_thread = Arc::clone(&seen);
        let release_for_thread = Arc::clone(&release);
        let reply = Arc::new(reply);

        let handle = std::thread::spawn(move || {
            listener
                .set_nonblocking(false)
                .expect("mock provider listener 保持阻塞模式");
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _addr)) = listener.accept() else {
                    return;
                };
                let body = read_http_request_body(&mut stream);
                requests_for_thread
                    .lock()
                    .expect("mock provider request lock poisoned")
                    .push(body);
                seen_for_thread.store(true, Ordering::SeqCst);
                match reply.as_ref() {
                    MockReply::JsonToolCall => {
                        write_http_json(&mut stream, &tool_call_completion_body());
                    }
                    MockReply::SseStall => {
                        // 只发响应头，随后保持连接直到用例收尾（驱动 provider 读通道的
                        // 取消观测：guard 每 tick 检查 stop_requested）。
                        let headers = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: keep-alive\r\n\r\n";
                        let _ = stream.write_all(headers.as_bytes());
                        let _ = stream.flush();
                        let deadline = Instant::now() + Duration::from_secs(5);
                        while !shutdown_for_thread.load(Ordering::SeqCst)
                            && !release_for_thread.load(Ordering::SeqCst)
                            && Instant::now() < deadline
                        {
                            std::thread::sleep(Duration::from_millis(2));
                        }
                    }
                }
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            requests,
            shutdown,
            seen,
            release,
            handle: Some(handle),
        }
    }

    /// 等待上游真正收到请求（确定性握手，禁止盲等固定时长）。
    fn wait_request(&self, timeout: Duration) -> bool {
        wait_until(
            || self.seen.load(Ordering::SeqCst),
            timeout.as_millis() as u64,
        )
    }

    /// 打开 SSE 停帧的释放闸门：mock 关闭连接 ⇒ in-band 循环恢复（真实错误返回路径）。
    fn release_stream(&self) {
        self.release.store(true, Ordering::SeqCst);
    }

    fn request_count(&self) -> usize {
        self.requests
            .lock()
            .expect("mock provider request lock poisoned")
            .len()
    }
}

impl Drop for MockProviderServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // 用一次空连接唤醒 accept，令 mock 线程及时退出。
        let _ = TcpStream::connect(self.addr.clone());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_http_request_body(stream: &mut TcpStream) -> String {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
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
        if let Some(header_end) = header_end {
            if buffer.len().saturating_sub(header_end) >= content_length {
                break;
            }
        }
    }
    let body_start = header_end.unwrap_or(buffer.len());
    String::from_utf8_lossy(&buffer[body_start..]).to_string()
}

fn write_http_json(stream: &mut TcpStream, body: &str) {
    let payload = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.as_bytes().len(),
        body
    );
    let _ = stream.write_all(payload.as_bytes());
    let _ = stream.flush();
}

fn tool_call_completion_body() -> String {
    serde_json::json!({
        "choices": [
            {
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [
                        {
                            "id": "call-wave64-1",
                            "type": "function",
                            "function": {
                                // 必须是 runtime 认识的**内建**工具名：否则 dispatch 前置校验
                                // 会以「capability registry 无此条目」直接返回 error（工具不执行，
                                // 也就无法构造「工具执行期用户取消」这一真实边界）。
                                "name": "Read",
                                "arguments": "{\"path\":\"wave64-acceptance-probe.txt\"}"
                            }
                        }
                    ]
                }
            }
        ],
        "usage": { "prompt_tokens": 3, "completion_tokens": 3, "total_tokens": 6 }
    })
    .to_string()
}

struct StaticProviderResolver {
    selection: ResolvedProviderSelection,
}

impl ProviderSelectionResolver for StaticProviderResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        self.selection.clone()
    }
}

fn provider_selection(base_url: String, supports_streaming: bool) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "wave64-test".to_string(),
        provider_name: "wave64-test".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "WAVE64_TEST_API_KEY".to_string(),
        api_key: Some("wave64-test-key".to_string()),
        model: "wave64-test-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 256,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(8_192),
            supports_tools: true,
            supports_streaming,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

/// 注入式工具执行器：模拟"用户在工具执行期点停止"——
/// 先发出**真实**的停止请求（与桌面 `stop_graph_run` 同一取消源），再返回 `aborted`。
struct UserCancellingAbortingExecutor {
    registry: ExecutionControlRegistry,
    turn_id: String,
    stop_issued: Arc<AtomicBool>,
}

impl ToolExecutor for UserCancellingAbortingExecutor {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        let response = self
            .registry
            .request_stop(&self.turn_id, StopReason::User);
        self.stop_issued.store(response.accepted, Ordering::SeqCst);
        ToolResult {
            tool_name: call.name.clone(),
            status: "aborted".to_string(),
            output: "{\"error\":{\"code\":\"tool_cancelled\",\"message\":\"cancelled by user\"}}"
                .to_string(),
            duration_ms: 1,
        }
    }
}

/// 注入式工具执行器：返回 `aborted` 但**不**发停止请求（真实失败/非取消边界）。
struct PlainAbortingExecutor;

impl ToolExecutor for PlainAbortingExecutor {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        ToolResult {
            tool_name: call.name.clone(),
            status: "aborted".to_string(),
            output: "{\"error\":{\"code\":\"tool_cancelled\",\"message\":\"aborted\"}}".to_string(),
            duration_ms: 1,
        }
    }
}

fn runtime_with(
    base_url: String,
    supports_streaming: bool,
    executor: Box<dyn ToolExecutor>,
) -> AgentRuntime {
    AgentRuntime::with_dependencies(
        isolated_session_store(),
        Box::new(StaticProviderResolver {
            selection: provider_selection(base_url, supports_streaming),
        }),
        executor,
        Box::new(LocalTurnPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    )
}

/// R2：工具执行期（工具返回 `aborted`）且用户已请求停止 ⇒ 终态必须是 `turn:cancelled`
/// （`stopReason="user"`），**禁止**派生 `turn:failed`；checkpoint 终态为 cancelled。
#[test]
fn r2_tool_aborted_during_user_stop_terminates_cancelled_not_failed() {
    // Arrange：本地回环 provider 返回一次工具调用；工具在执行中发出真实 stop 再返回 aborted。
    let server = MockProviderServer::start(MockReply::JsonToolCall);
    let registry = ExecutionControlRegistry::new();
    let turn_id = "turn-wave64-r2a";
    let session_id = "session-wave64-r2a";
    registry.register_turn(turn_id, Some(session_id), None);
    let stop_issued = Arc::new(AtomicBool::new(false));
    let runtime = runtime_with(
        server.base_url.clone(),
        false,
        Box::new(UserCancellingAbortingExecutor {
            registry: registry.clone(),
            turn_id: turn_id.to_string(),
            stop_issued: Arc::clone(&stop_issued),
        }),
    );
    let sink = RecordingSink::new();

    // Act
    runtime.start_turn_stream_with_control(
        &sink,
        &registry,
        turn_id.to_string(),
        test_turn_input(session_id, "跑一个会被用户取消的工具"),
    );

    // Assert：夹具自检 —— 工具确实拿到了被接受的停止请求（否则本用例无判别力）
    assert!(
        stop_issued.load(Ordering::SeqCst),
        "R2 夹具自检：工具执行期必须真的发出了被接受的停止请求（事件序列 = {:?}）",
        sink.names()
    );
    assert!(
        server.request_count() >= 1,
        "R2 夹具自检：必须真实打到本地回环 provider（模拟真实桌面上游）"
    );
    assert!(
        !sink.events_named("turn:cancelled").is_empty(),
        "R2：工具 aborted + 用户取消 ⇒ 终态必须是 turn:cancelled，实际事件序列 = {:?}",
        sink.names()
    );
    assert!(
        sink.events_named("turn:failed").is_empty(),
        "R2：用户取消**禁止**派生 turn:failed，实际事件序列 = {:?}",
        sink.names()
    );
    let cancelled = sink.events_named("turn:cancelled");
    assert_eq!(
        cancelled[0].stop_reason.as_deref(),
        Some("user"),
        "R2：turn:cancelled 必须带 stopReason=\"user\""
    );
    let checkpoint = registry
        .load_checkpoint(Some(turn_id), None)
        .expect("R2：turn 必须有 checkpoint");
    assert_eq!(checkpoint.status, "cancelled", "R2：checkpoint 终态必须是 cancelled");
}

/// R2：provider 读通道以 `provider_stream_cancelled` 结束且用户已请求停止 ⇒ 归 `turn:cancelled`，
/// 禁止 `turn:failed`；checkpoint 终态为 cancelled。
#[test]
fn r2_provider_stream_cancelled_terminates_cancelled_not_failed() {
    // Arrange：SSE 停帧的上游 + 从独立线程按下停止（模拟桌面用户点停止）
    let server = MockProviderServer::start(MockReply::SseStall);
    let registry = ExecutionControlRegistry::new();
    let turn_id = "turn-wave64-r2b";
    let session_id = "session-wave64-r2b";
    registry.register_turn(turn_id, Some(session_id), None);
    let runtime = runtime_with(server.base_url.clone(), true, Box::new(PlainAbortingExecutor));
    let sink = RecordingSink::new();

    let registry_for_stopper = registry.clone();
    let turn_for_stopper = turn_id.to_string();
    let (request_seen_tx, request_seen_rx) = mpsc::channel::<()>();
    let stopper = std::thread::spawn(move || {
        // 等上游**真实收到请求**后再按停止（确定性握手，不靠盲等固定时长）。
        let _ = request_seen_rx.recv_timeout(Duration::from_millis(WAIT_BUDGET_MS));
        registry_for_stopper.request_stop(&turn_for_stopper, StopReason::User)
    });
    let watcher = std::thread::spawn({
        let requests = Arc::clone(&server.requests);
        move || {
            let seen = wait_until(
                || {
                    !requests
                        .lock()
                        .expect("mock provider request lock poisoned")
                        .is_empty()
                },
                WAIT_BUDGET_MS,
            );
            if seen {
                let _ = request_seen_tx.send(());
            }
        }
    });

    // Act
    runtime.start_turn_stream_with_control(
        &sink,
        &registry,
        turn_id.to_string(),
        test_turn_input(session_id, "跑一个会在 provider 读通道被取消的轮次"),
    );

    let stop_response = stopper.join().expect("stopper 线程不得 panic");
    watcher.join().expect("watcher 线程不得 panic");
    assert!(
        stop_response.accepted,
        "R2 夹具自检：上游读通道在途时按下的停止必须被接受（state={}）",
        stop_response.state
    );

    // Assert
    assert!(
        server.request_count() >= 1,
        "R2 夹具自检：必须真实打到本地回环 provider"
    );
    assert!(
        !sink.events_named("turn:cancelled").is_empty(),
        "R2：provider_stream_cancelled + 用户取消 ⇒ 终态必须是 turn:cancelled，实际事件序列 = {:?}",
        sink.names()
    );
    assert!(
        sink.events_named("turn:failed").is_empty(),
        "R2：用户取消**禁止**派生 turn:failed，实际事件序列 = {:?}",
        sink.names()
    );
    let checkpoint = registry
        .load_checkpoint(Some(turn_id), None)
        .expect("R2：turn 必须有 checkpoint");
    assert_eq!(checkpoint.status, "cancelled", "R2：checkpoint 终态必须是 cancelled");
}

/// R2 回归边界：无停止请求的真实失败行为不变 —— 工具返回 error（非用户取消）⇒ 仍 `turn:failed`。
#[test]
fn r2_non_user_failure_still_terminates_failed() {
    // Arrange：上游直接返回"无工具调用且无文本"的非法决策 ⇒ 决策失败分支
    struct ErroringExecutor;
    impl ToolExecutor for ErroringExecutor {
        fn execute(&self, call: &ToolCall) -> ToolResult {
            ToolResult {
                tool_name: call.name.clone(),
                status: ToolExecutionStatus::Error.as_legacy_status().to_string(),
                output: "{\"error\":{\"code\":\"boom\"}}".to_string(),
                duration_ms: 1,
            }
        }
    }

    let server = MockProviderServer::start(MockReply::JsonToolCall);
    let registry = ExecutionControlRegistry::new();
    let turn_id = "turn-wave64-r2c";
    let session_id = "session-wave64-r2c";
    registry.register_turn(turn_id, Some(session_id), None);
    let runtime = runtime_with(server.base_url.clone(), false, Box::new(ErroringExecutor));
    let sink = RecordingSink::new();

    // Act
    runtime.start_turn_stream_with_control(
        &sink,
        &registry,
        turn_id.to_string(),
        test_turn_input(session_id, "无停止请求的真实失败"),
    );

    // Assert：没有停止请求 ⇒ 不得被归类为 cancelled（事件序列中不得出现 cancelled）
    assert!(
        sink.events_named("turn:cancelled").is_empty(),
        "R2 回归：无停止请求的失败不得被归类为 turn:cancelled，实际事件序列 = {:?}",
        sink.names()
    );
}

// ---------------------------------------------------------------------------
// R3：C5 宽限监督已 escalate 之后，in-band 循环收口不得追加第二条终态
// ---------------------------------------------------------------------------

/// R3 专用宽限期：必须**先于** provider 读通道的取消观测节拍（`DEFAULT_CANCEL_TICK = 250ms`）
/// 到点，才能构造确定性的「escalation 先、in-band 后」时序（不依赖任何真实 2s 宽限）。
const TEST_GRACE_MS_R3: u64 = 40;

/// R3（L3-A BLOCKING）：C5 宽限监督已经 escalate（checkpoint 置 cancelled + 派发一条
/// `turn:cancelled`）之后，in-band 流式循环恢复并收口（命中停止位 / 真实错误返回）
/// ⇒ **必须仍只派发恰好一条 `turn:cancelled`**，且不得被后续 `turn:failed` 覆盖或追加。
///
/// 确定性时序（无盲等）：
/// 1. 上游 mock 收到请求即置 `seen`（夹具握手点：in-band 循环已进入 provider 读通道）；
/// 2. 主线程 `stop_turn` ⇒ C2 派发 `turn:stop_requested` + 启动 C5 宽限监督（注入 40ms）；
/// 3. 轮询到宽限监督派发的**第一条** `turn:cancelled`（checkpoint 已 cancelled）；
/// 4. 打开 mock 停帧释放闸门 ⇒ in-band 循环恢复（EOF/取消码）并尝试收口；
/// 5. 断言 `turn:cancelled` 恰一条、`turn:failed` 零条、checkpoint 终态 cancelled。
#[test]
fn r3_in_band_closure_after_grace_escalation_keeps_single_cancelled_terminal() {
    // Arrange：本地回环 provider 停帧（可控唤醒点）+ 注入毫秒级宽限监督
    let server = MockProviderServer::start(MockReply::SseStall);
    let registry = ExecutionControlRegistry::new();
    let turn_id = "turn-wave64-r3";
    let session_id = "session-wave64-r3";
    registry.register_turn(turn_id, Some(session_id), None);

    // 控制面只用于「注册宿主事件出口（sink）+ C2/C5 入口」；宽限监督用测试注入小值。
    let plane_registry = registry.clone();
    let control_plane = HostControlPlaneBuilder::new()
        .runtime(
            AgentRuntimeBuilder::new()
                .session_store(isolated_session_store())
                .build(),
        )
        .graph_run_store(GraphRunStore::new())
        .execution_control(plane_registry.clone())
        .stop_escalation(Arc::new(StopEscalation::with_registry(
            StopEscalationConfig::for_test(TEST_GRACE_MS_R3, TEST_POLL_TICK_MS),
            plane_registry,
        )))
        .build();

    let sink = Arc::new(RecordingSink::new());
    control_plane.register_turn_event_sink(turn_id, Arc::clone(&sink));

    // in-band 流式轮（真实运行时 + 本地回环 provider）在独立线程执行，主线程负责停止与唤醒。
    let runtime = runtime_with(server.base_url.clone(), true, Box::new(PlainAbortingExecutor));
    let registry_for_worker = registry.clone();
    let sink_for_worker = Arc::clone(&sink);
    let turn_id_for_worker = turn_id.to_string();
    let worker = std::thread::spawn(move || {
        runtime.start_turn_stream_with_control(
            sink_for_worker.as_ref(),
            &registry_for_worker,
            turn_id_for_worker,
            test_turn_input(session_id, "跨宽限期收口的轮次"),
        );
    });

    // Act ①：确认 in-band 循环已进入 provider 读通道（确定性握手，非盲等）
    assert!(
        server.wait_request(Duration::from_millis(WAIT_BUDGET_MS)),
        "R3 夹具自检：in-band 循环必须真实打到本地回环 provider"
    );
    let stop_response = control_plane.stop_turn(StopTurnCommand {
        turn_id: turn_id.to_string(),
        stop_reason: Some(StopReason::User),
    });
    assert!(
        stop_response.accepted,
        "R3 夹具自检：活跃 turn 的停止请求必须被接受（state={}）",
        stop_response.state
    );

    // Act ②：等宽限监督 escalate（第一条 turn:cancelled 已落盘）
    let escalated = wait_until(
        || sink.events_named("turn:cancelled").len() >= 1,
        WAIT_BUDGET_MS,
    );
    assert!(
        escalated,
        "R3 夹具自检：宽限期（注入 {TEST_GRACE_MS_R3}ms）到点后监督者必须收口并派发 turn:cancelled；实际事件序列 = {:?}",
        sink.names()
    );
    let escalated_checkpoint = registry
        .load_checkpoint(Some(turn_id), None)
        .expect("R3：活跃 turn 必须有 checkpoint");
    assert_eq!(
        escalated_checkpoint.status, "cancelled",
        "R3 夹具自检：escalate 之后 checkpoint 必须已是 cancelled"
    );

    // Act ③：打开闸门，令 in-band 循环恢复并收口（命中停止位 / 真实错误返回）
    server.release_stream();
    assert!(
        wait_until(|| worker.is_finished(), WAIT_BUDGET_MS),
        "R3：释放闸门后 in-band 循环必须在 {WAIT_BUDGET_MS}ms 内收口（事件序列 = {:?}）",
        sink.names()
    );
    worker.join().expect("in-band 线程不得 panic");

    // Assert：终态**恰好一次**、且不得被 failed 追加
    let cancelled = sink.events_named("turn:cancelled");
    assert_eq!(
        cancelled.len(),
        1,
        "R3：C5 escalation 之后 in-band 收口不得再派发第二条终态 —— turn:cancelled 必须恰好一条；实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        sink.events_named("turn:failed").len(),
        0,
        "R3：已 cancelled 的轮次不得被后续 turn:failed 覆盖/追加；实际事件序列 = {:?}",
        sink.names()
    );
    let final_checkpoint = registry
        .load_checkpoint(Some(turn_id), None)
        .expect("R3：turn 必须有 checkpoint");
    assert_eq!(
        final_checkpoint.status, "cancelled",
        "R3：in-band 收口后 checkpoint 终态必须仍是 cancelled"
    );
}
