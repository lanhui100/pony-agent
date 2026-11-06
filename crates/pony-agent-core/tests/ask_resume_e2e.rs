//! Ask「回答后本轮不继续」——黑盒端到端验收（独立 Test Agent 红相，L2）。
//!
//! 契约真值源：`.dev-team/plan-ask-resume.md`（frozen）§2 C1/C2/C3 + §3 T1~T5 + §1 INV-1..INV-5。
//! 本文件是**唯一**写域：不引用任何 `cfg(test)` 私有夹具，全部经 public 入口驱动。
//!
//! 覆盖与判据：
//! - **T1**（C1+C2）：TUI 真实入口 `HostControlPlane::start_turn_stream` 跑一轮 ⇒ mock provider
//!   返回 `Ask` 工具调用 ⇒ `turn:suspended`；`answer_ask_in_session` + `graph_resume_ask` 后经
//!   `start_turn_stream` 跑续轮 ⇒ **恰好一条** `turn:completed`，正文 == mock 第二段 assistant 文本，
//!   且该轮仍落在**同一个** graph run 上（run 总数仍为 1，id 不变）。
//! - **T2**（C1）：挂起请求持有真实 run 身份（`list_pending_asks(Some(session))[0].runId` 为 Some）、run phase ==
//!   `WaitingUser`、`graph_list_ask_waits(run_id)` 恰一条且 `requestId`/`expectedVersion` 与 pending 对齐、
//!   事件里 `turn:suspended` 恰 1 条且 `turn:failed` 恰 0 条。
//! - **T3**（C3，红线）：无 run 身份的旧入口 `AgentRuntime::start_turn_stream_with_control` +
//!   governed executor 返回 `Ask` ⇒ 终态必须是 `turn:failed` 且 error 以 `ask_resume_unavailable`
//!   开头，`turn:suspended` **恰 0 条**（不可恢复时不得出现无终态挂起）。
//! - **T4**（INV-1）：未作答直接 `graph_resume_ask` 必须 Err；`answer_ask_in_session` 用过期
//!   `expected_version` 必须 Err；两次 Err 之后仍可用正确版本消费（CAS 未被破坏）。
//! - **T5**（INV-3+INV-4）：挂起时 provider 只被请求 1 次（不回灌）；续轮的第 2 次请求体必须包含
//!   原 Ask `call_id`（`call-ask-e2e-1`）对应的终结 tool 结果，且**不得**包含字面串
//!   `control_outcome_pending`。
//!
//! 红相说明（写于实施前，如实记录当前基线）：C1（`start_turn_stream` 内建真实 graph run）与
//! C3（`ask_resume_unavailable` fail-closed）尚未实现，故 **T1/T2/T4/T5 现在必然失败**
//! （最先失败在「挂起请求没有 run 身份」这一事实），**T3 也失败**（当前实现对「Interaction 且无
//! run 身份」仍下发裸 `turn:suspended`）。实施完成后本文件转绿且此后只读。
//!
//! R3 契约迁移（2026-11-06，Lead 裁决 / ADR 0074）：未决 Ask 的读取与作答入口改为**会话作用域**
//! 强绑定 —— `list_pending_asks(Some(session_id))`、`answer_ask_in_session(session_id, …)`；
//! 旧的无作用域 `list_pending_asks(None)` / `answer_ask(…)` 现为 fail-closed（`ask_scope_required`）。
//! 本次迁移**只换入口**，T1~T5 的断言与事件语义逐条保持原样（未放松任何一条）。
//!
//! 纪律（test-expert L2）：真实断言、AAA、命名带判据号；禁止 `#[ignore]` / catch 吞异常 / 空断言；
//! 跨线程/异步等待一律显式条件轮询（`wait_until`）+ 硬上界，绝不裸 sleep 盲等。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::control_plane::{
    GraphRunQuery, HostControlPlane, HostControlPlaneBuilder, StartTurnStreamCommand,
};
use pony_agent_core::agent::execution_control::ExecutionControlRegistry;
use pony_agent_core::agent::graph::{GraphRunPhase, GraphRunStore};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntime, AgentRuntimeBuilder, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::SessionStore;
use pony_agent_core::agent::turn_flow::TurnEventSink;

/// mock provider 第 1 次应答里 Ask 工具调用的 id（T5 以此为「终结 tool 结果」的对拍键）。
const ASK_CALL_ID: &str = "call-ask-e2e-1";
/// Ask 工具调用的问句（`list_pending_asks()[0].prompt` 应原样上浮）。
const ASK_QUESTION: &str = "继续吗？";
/// mock provider 第 2 次应答的 assistant 文本（T1 断言 `turn:completed.text` 与之逐字相等）。
const CONTINUATION_TEXT: &str = "续轮完成：已按你的回答继续执行。";
/// C2 规定的宿主续轮提示（非空，否则 `prepare_turn` 的 `reject_empty` 会拒绝）。
const CONTINUATION_PROMPT: &str = "（宿主续轮）请基于刚才的 Ask 回答继续本轮工作。";
/// 显式条件轮询的硬上界（毫秒）。
const WAIT_BUDGET_MS: u64 = 10_000;

/// 集成测试（`tests/*.rs`）编译 crate 时不启用 `cfg(test)`，`SessionStore::new()` 会走生产分支
/// 指向用户真实数据目录。这里在**任何**控制面/运行时构造之前（`Once` 保证顺序）把数据目录基准
/// 重定向到本测试二进制独有的临时沙箱，确保集成测试绝不读写生产数据（与
/// `stop_graph_run_wave64.rs` 同法）。
static ISOLATE_PROCESS_DATA_DIR: Once = Once::new();

fn isolate_process_data_dir() {
    ISOLATE_PROCESS_DATA_DIR.call_once(|| {
        let unique = format!(
            "pony-agent-it-ask-resume-{}-{}",
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

/// 每个用例独占的工作区根（Ask 只做治理判定，不落任何业务文件）。
fn isolated_workspace(test_name: &str) -> std::path::PathBuf {
    isolate_process_data_dir();
    let dir = std::env::temp_dir().join(format!(
        "pony-agent-ask-resume-ws-{}-{}",
        std::process::id(),
        test_name
    ));
    std::fs::create_dir_all(&dir).expect("create isolated workspace dir");
    dir
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

/// 用例串行闸门（防 Flaky，非断言）。
///
/// R3 起未决 Ask 的唯一真值源是持久化 store（`sessions.db`，ADR 0074）；同进程的用例共享同一
/// 沙箱库，并行跑时会互相触发 `database is locked` ⇒ 挂起记录可能未落盘 ⇒ 断言随机变红。
/// 这里只串行**夹具**（不含任何判定），断言强度不变。
static SERIAL_TEST_GUARD: Mutex<()> = Mutex::new(());

fn serial_guard() -> std::sync::MutexGuard<'static, ()> {
    SERIAL_TEST_GUARD
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

// ---------------------------------------------------------------------------
// 本地回环 mock provider（队列式应答，非流式 JSON：最容易构造 tool_call）
// ---------------------------------------------------------------------------

/// mock 的一次应答（按请求到达顺序出队）。
enum MockReply {
    /// 非流式决策：一次 `Ask` 工具调用（`stream: false` 路径）。
    AskToolCall,
    /// 非流式完结：纯文本 assistant 应答（无 tool_calls）。
    Text(String),
}

struct MockProviderServer {
    base_url: String,
    addr: String,
    /// 按到达顺序记录**原始请求体**（断言用）。
    requests: Arc<Mutex<Vec<String>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockProviderServer {
    fn start(replies: Vec<MockReply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock provider 必须能绑定回环端口");
        let port = listener.local_addr().expect("mock provider 地址").port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let replies = Arc::new(Mutex::new(VecDeque::from(replies)));
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests_for_thread = Arc::clone(&requests);
        let replies_for_thread = Arc::clone(&replies);
        let shutdown_for_thread = Arc::clone(&shutdown);

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
                let reply = replies_for_thread
                    .lock()
                    .expect("mock provider reply queue poisoned")
                    .pop_front();
                let payload = match reply {
                    Some(MockReply::AskToolCall) => ask_tool_call_body(ASK_CALL_ID),
                    Some(MockReply::Text(text)) => text_completion_body(&text),
                    // 队列耗尽：返回一段可辨识文本，绝不 hang / 绝不伪造 tool_call，
                    // 让多余请求在断言里显形（T5 会断言请求次数恰好 2）。
                    None => text_completion_body("mock-provider-queue-exhausted"),
                };
                write_http_json(&mut stream, &payload);
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            requests,
            shutdown,
            handle: Some(handle),
        }
    }

    fn request_count(&self) -> usize {
        self.requests
            .lock()
            .expect("mock provider request lock poisoned")
            .len()
    }

    fn request_body(&self, index: usize) -> Option<String> {
        self.requests
            .lock()
            .expect("mock provider request lock poisoned")
            .get(index)
            .cloned()
    }

    /// 等待上游真正累计到 `expected` 次请求（确定性握手，禁止盲等固定时长）。
    fn wait_request_count(&self, expected: usize) -> bool {
        wait_until(|| self.request_count() >= expected, WAIT_BUDGET_MS)
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

/// 第 1 段应答：assistant 发起一次 `Ask` 工具调用（参数与 TUI 真实形态一致）。
fn ask_tool_call_body(call_id: &str) -> String {
    let arguments = json!({
        "question": ASK_QUESTION,
        "options": ["继续"],
        "description": "it",
    })
    .to_string();
    json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": {
                        "name": "Ask",
                        "arguments": arguments,
                    }
                }]
            }
        }],
        "usage": { "prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10 }
    })
    .to_string()
}

/// 第 2 段应答：assistant 纯文本完结（续轮的真实产出）。
fn text_completion_body(text: &str) -> String {
    json!({
        "choices": [{
            "message": { "role": "assistant", "content": text }
        }],
        "usage": { "prompt_tokens": 7, "completion_tokens": 7, "total_tokens": 14 }
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

/// 非流式（`supports_streaming = false`）是最容易构造 tool_call 的路径：
/// 决策与续轮都走同步 JSON，mock 只需按队列回一段 `choices[0].message`。
fn provider_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "ask-resume-test".to_string(),
        provider_name: "ask-resume-test".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "ASK_RESUME_TEST_API_KEY".to_string(),
        api_key: Some("ask-resume-test-key".to_string()),
        model: "ask-resume-test-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 256,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(8_192),
            supports_tools: true,
            supports_streaming: false,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

/// TUI 真实入口的控制面：隔离会话库 + 内存 graph run store + 默认 governed executor
/// （`AgentRuntimeBuilder::new()` 已装配 `build_governed_executor`，Ask 治理链路依赖它）。
fn control_plane_with_mock_provider(base_url: String, workspace: &Path) -> HostControlPlane {
    let runtime = AgentRuntimeBuilder::new()
        .session_store(isolated_session_store())
        .provider_resolver(Box::new(StaticProviderResolver {
            selection: provider_selection(base_url),
        }))
        .workspace_root(workspace)
        .build();
    HostControlPlaneBuilder::new()
        .runtime(runtime)
        .graph_run_store(GraphRunStore::new())
        .build()
}

fn runtime_with_mock_provider(base_url: String, workspace: &Path) -> AgentRuntime {
    AgentRuntimeBuilder::new()
        .session_store(isolated_session_store())
        .provider_resolver(Box::new(StaticProviderResolver {
            selection: provider_selection(base_url),
        }))
        .workspace_root(workspace)
        .build()
}

// ---------------------------------------------------------------------------
// 断言辅助（JSON 投影读取 + 带判据号的消息）
// ---------------------------------------------------------------------------

/// 该会话作用域下的未决 Ask 快照（R3：作用域强绑定，无作用域调用是 fail-closed 空数组）。
fn pending_asks(control_plane: &HostControlPlane, session_id: &str) -> Vec<Value> {
    match control_plane.list_pending_asks(Some(session_id)) {
        Value::Array(items) => items,
        other => panic!("list_pending_asks 必须返回 JSON 数组，实际 = {other}"),
    }
}

fn json_str(request: &Value, key: &str, case: &str) -> String {
    request
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{case}：挂起请求必须带字符串字段 `{key}`，实际 = {request}"))
        .to_string()
}

fn json_u64(request: &Value, key: &str, case: &str) -> u64 {
    request
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{case}：挂起请求必须带无符号整数字段 `{key}`，实际 = {request}"))
}

/// 挂起请求**必须**持有的 run 身份（C1）。缺失即 `{case}` 判据失败。
fn require_run_id(request: &Value, case: &str) -> String {
    request
        .get("runId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            panic!(
                "{case}：挂起请求必须持久化真实 run 身份（`runId` 为 Some），否则 graph ask-wait \
                 无法绑定、作答后无法续轮；实际 = {request}"
            )
        })
}

fn expect_ok<T>(result: Result<T, String>, case: &str) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("{case}：必须成功，实际失败 = {error}"),
    }
}

fn expect_err<T>(result: Result<T, String>, case: &str) -> String {
    match result {
        Ok(_) => panic!("{case}：必须 fail-closed 返回 Err，实际返回了成功值"),
        Err(error) => {
            assert!(
                !error.trim().is_empty(),
                "{case}：Err 必须携带可读错误信息，不得为空串"
            );
            error
        }
    }
}

/// 夹具：经 TUI 真实入口跑一轮，直到 Ask 挂起；返回该 pending ask 的 JSON 快照。
fn run_turn_until_ask_suspension(
    control_plane: &HostControlPlane,
    sink: &RecordingSink,
    session_id: &str,
    turn_id: &str,
    case: &str,
) -> Value {
    control_plane.start_turn_stream(
        sink,
        StartTurnStreamCommand {
            turn_id: turn_id.to_string(),
            input: test_turn_input(session_id, "先向我确认是否继续，再继续执行"),
        },
    );

    let names = sink.names();
    assert!(
        names.iter().any(|name| name == "turn:suspended"),
        "{case}：Ask 工具调用必须让本轮以 turn:suspended 收口（可恢复挂起），实际事件序列 = {names:?}"
    );

    let asks = pending_asks(control_plane, session_id);
    assert_eq!(
        asks.len(),
        1,
        "{case}：挂起后必须恰好一条 pending ask，实际 = {asks:?}"
    );
    asks[0].clone()
}

/// C2 的宿主续轮：沿用 `start_turn_stream` + 固定续轮提示 + 与挂起轮一致的 session。
fn run_continuation_turn(
    control_plane: &HostControlPlane,
    sink: &RecordingSink,
    session_id: &str,
    turn_id: &str,
    case: &str,
) {
    control_plane.start_turn_stream(
        sink,
        StartTurnStreamCommand {
            turn_id: turn_id.to_string(),
            input: test_turn_input(session_id, CONTINUATION_PROMPT),
        },
    );
    let names = sink.names();
    assert!(
        names.iter().any(|name| name == "turn:completed"),
        "{case}：续轮必须产出 turn:completed（作答后本轮真的继续），实际事件序列 = {names:?}"
    );
}

// ---------------------------------------------------------------------------
// T1：作答后同一 run 续轮产出下一段 assistant 输出（C1 + C2）
// ---------------------------------------------------------------------------

#[test]
fn t1_ask_answer_resumes_the_same_run_and_produces_next_assistant_output() {
    let _serial = serial_guard();

    // Arrange：mock provider 第 1 次回 Ask 工具调用，第 2 次回纯文本续轮产出。
    let server = MockProviderServer::start(vec![
        MockReply::AskToolCall,
        MockReply::Text(CONTINUATION_TEXT.to_string()),
    ]);
    let workspace = isolated_workspace("t1");
    let control_plane = control_plane_with_mock_provider(server.base_url.clone(), &workspace);
    let sink = RecordingSink::new();
    let session_id = "session-ask-e2e-t1";

    // Act ①：TUI 真实入口跑一轮 ⇒ 模型发起 Ask ⇒ 本轮挂起。
    let pending = run_turn_until_ask_suspension(
        &control_plane,
        &sink,
        session_id,
        "turn-ask-e2e-t1-1",
        "T1",
    );
    let request_id = json_str(&pending, "requestId", "T1");
    let expected_version = json_u64(&pending, "version", "T1");
    let run_id = require_run_id(&pending, "T1");

    // Act ②：宿主作答（CAS 消费）→ 恢复 graph ask-wait（一次性注入终结 tool 结果）。
    expect_ok(
        control_plane.answer_ask_in_session(session_id, &request_id, expected_version, json!("继续")),
        "T1：answer_ask 必须以当前版本 CAS 消费挂起请求",
    );
    let resumed = expect_ok(
        control_plane.graph_resume_ask(&run_id, &request_id, expected_version, json!("继续")),
        "T1：作答后 graph_resume_ask 必须把该 run 的 ask-wait 恢复为可续",
    );
    assert_eq!(
        resumed.get("callId").and_then(Value::as_str),
        Some(ASK_CALL_ID),
        "T1：graph_resume_ask 必须回带原 Ask 的 call_id（终结 tool 结果的对拍键），实际 = {resumed}"
    );

    // Act ③：续轮（C2：沿用 start_turn_stream + 固定续轮提示）。
    run_continuation_turn(
        &control_plane,
        &sink,
        session_id,
        "turn-ask-e2e-t1-2",
        "T1",
    );

    // Assert ①：续轮产出**恰好一条** turn:completed，正文 == mock 第二段文本。
    let completed = sink.events_named("turn:completed");
    assert_eq!(
        completed.len(),
        1,
        "T1：作答前的挂起轮不得产出 turn:completed，续轮必须产出**恰好一条** turn:completed；\
         实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        completed[0].text.as_deref(),
        Some(CONTINUATION_TEXT),
        "T1：turn:completed 的正文必须等于 mock 的第二段 assistant 文本（作答后模型真的继续了）"
    );

    // Assert ②：续轮落在**同一个** graph run 上（C1/C2 的 run 复用判据）。
    let runs = control_plane.list_graph_runs();
    assert_eq!(
        runs.len(),
        1,
        "T1：挂起轮与续轮必须复用同一个 graph run（不得为续轮新建 run），实际 runs = {:?}",
        runs.iter().map(|run| run.id.clone()).collect::<Vec<_>>()
    );
    assert_eq!(
        runs[0].id, run_id,
        "T1：续轮所在的 run 必须就是挂起时被绑定 ask-wait 的那个 run"
    );
}

// ---------------------------------------------------------------------------
// T2：挂起轮持有真实 run 身份 + graph ask-wait 对齐 + 零 failed（C1）
// ---------------------------------------------------------------------------

#[test]
fn t2_ask_suspension_binds_graph_run_and_persists_run_id_on_pending_request() {
    let _serial = serial_guard();

    // Arrange
    let server = MockProviderServer::start(vec![MockReply::AskToolCall]);
    let workspace = isolated_workspace("t2");
    let control_plane = control_plane_with_mock_provider(server.base_url.clone(), &workspace);
    let sink = RecordingSink::new();
    let session_id = "session-ask-e2e-t2";

    // Act
    let pending = run_turn_until_ask_suspension(
        &control_plane,
        &sink,
        session_id,
        "turn-ask-e2e-t2-1",
        "T2",
    );

    // Assert ①：pending ask 持久化了真实 run 身份。
    let run_id = require_run_id(&pending, "T2");
    assert_eq!(
        pending.get("requestKind").and_then(Value::as_str),
        Some("interaction"),
        "T2：Ask 必须以 Interaction 形态挂起，实际 = {pending}"
    );
    assert_eq!(
        pending.get("state").and_then(Value::as_str),
        Some("pending"),
        "T2：挂起后请求必须处于 pending 状态（可被作答消费），实际 = {pending}"
    );
    assert_eq!(
        pending.get("prompt").and_then(Value::as_str),
        Some(ASK_QUESTION),
        "T2：pending ask 必须原样上浮模型给出的问句，实际 = {pending}"
    );

    // Assert ②：该 run 的 phase 必须是 WaitingUser。
    let run = control_plane
        .load_graph_run(GraphRunQuery {
            run_id: Some(run_id.clone()),
        })
        .unwrap_or_else(|| panic!("T2：挂起请求的 runId `{run_id}` 必须能在 graph store 中读到"));
    assert_eq!(
        run.phase,
        GraphRunPhase::WaitingUser,
        "T2：Ask 挂起必须把 run 置为 WaitingUser，实际 phase = {:?}",
        run.phase
    );

    // Assert ③：graph ask-wait 恰一条，且 requestId / expectedVersion 与 pending 对齐。
    let waits = match control_plane.graph_list_ask_waits(&run_id) {
        Value::Array(waits) => waits,
        other => panic!("T2：graph_list_ask_waits 必须返回 JSON 数组，实际 = {other}"),
    };
    assert_eq!(
        waits.len(),
        1,
        "T2：挂起 run 必须恰好绑定一条 ask-wait（否则作答后无法恢复），实际 = {waits:?}"
    );
    assert_eq!(
        waits[0].get("requestId").and_then(Value::as_str),
        Some(json_str(&pending, "requestId", "T2").as_str()),
        "T2：ask-wait 的 requestId 必须与 pending ask 对齐，实际 = {waits:?}"
    );
    assert_eq!(
        waits[0].get("expectedVersion").and_then(Value::as_u64),
        Some(json_u64(&pending, "version", "T2")),
        "T2：ask-wait 的 expectedVersion 必须是 pending ask 的 CAS 版本，实际 = {waits:?}"
    );
    assert_eq!(
        waits[0].get("callId").and_then(Value::as_str),
        Some(ASK_CALL_ID),
        "T2：ask-wait 的 callId 必须是原 Ask 工具调用 id（续轮注入的对拍键），实际 = {waits:?}"
    );

    // Assert ④：事件语义 —— suspended 恰 1 条，failed 恰 0 条（可恢复挂起不是失败）。
    assert_eq!(
        sink.events_named("turn:suspended").len(),
        1,
        "T2：可恢复的 Ask 挂起必须派发**恰好一条** turn:suspended，实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        sink.events_named("turn:failed").len(),
        0,
        "T2：可恢复的 Ask 挂起**禁止**派生 turn:failed，实际事件序列 = {:?}",
        sink.names()
    );
}

// ---------------------------------------------------------------------------
// T3：无 run 身份 ⇒ fail-closed（红线：不得出现无终态 suspended）（C3）
// ---------------------------------------------------------------------------

#[test]
fn t3_ask_without_run_identity_fails_closed_instead_of_silent_suspend() {
    let _serial = serial_guard();

    // Arrange：旧入口（无 run 事实）+ governed executor + mock provider 返回 Ask 工具调用。
    let server = MockProviderServer::start(vec![MockReply::AskToolCall]);
    let workspace = isolated_workspace("t3");
    let runtime = runtime_with_mock_provider(server.base_url.clone(), &workspace);
    let sink = RecordingSink::new();
    let control = ExecutionControlRegistry::new();
    let turn_id = "turn-ask-e2e-t3-1";
    let session_id = "session-ask-e2e-t3";
    control.register_turn(turn_id, Some(session_id), None);

    // Act：`start_turn_stream_with_control` 内部硬编码 `RunTurnFacts::default()`，无 run 身份。
    runtime.start_turn_stream_with_control(
        &sink,
        &control,
        turn_id.to_string(),
        test_turn_input(session_id, "没有 run 身份时也必须给出可读失败终态"),
    );

    // Assert ①：终态必须是 turn:failed（恰一条），不得静默挂起。
    let failed = sink.events_named("turn:failed");
    assert_eq!(
        failed.len(),
        1,
        "T3：Ask 无 run 身份时**必须**以 turn:failed 收口（fail-closed），实际事件序列 = {:?}",
        sink.names()
    );

    // Assert ②：error 必须以 ask_resume_unavailable 开头（可机器判别的错误码）。
    let error = failed[0].error.as_deref().unwrap_or_default();
    assert!(
        error.starts_with("ask_resume_unavailable"),
        "T3：失败终态的 error 必须以 `ask_resume_unavailable` 开头，实际 = {error:?}"
    );

    // Assert ③（红线）：不可恢复时**绝不**允许出现无终态 suspended。
    assert_eq!(
        sink.events_named("turn:suspended").len(),
        0,
        "T3 红线：不可恢复的 Ask 不得下发无终态 turn:suspended，实际事件序列 = {:?}",
        sink.names()
    );
}

// ---------------------------------------------------------------------------
// T4：CAS 唯一入口不变量（INV-1）
// ---------------------------------------------------------------------------

#[test]
fn t4_resume_requires_cas_consumed_request() {
    let _serial = serial_guard();

    // Arrange：先真实挂起一条 Ask。
    let server = MockProviderServer::start(vec![MockReply::AskToolCall]);
    let workspace = isolated_workspace("t4");
    let control_plane = control_plane_with_mock_provider(server.base_url.clone(), &workspace);
    let sink = RecordingSink::new();
    let session_id = "session-ask-e2e-t4";
    let pending = run_turn_until_ask_suspension(
        &control_plane,
        &sink,
        session_id,
        "turn-ask-e2e-t4-1",
        "T4",
    );
    let request_id = json_str(&pending, "requestId", "T4");
    let version = json_u64(&pending, "version", "T4");
    let run_id = require_run_id(&pending, "T4");

    // Act ①：未 CAS 消费直接 resume ⇒ 必须 Err。
    let premature = control_plane.graph_resume_ask(&run_id, &request_id, version, json!("继续"));
    let premature_error = expect_err(
        premature,
        "T4：未经 answer_ask 消费的挂起请求不得被 graph_resume_ask 放行（CAS 唯一入口）",
    );
    assert!(
        !premature_error.trim().is_empty(),
        "T4：提前 resume 的 Err 必须可读，实际 = {premature_error:?}"
    );

    // Assert ①：提前 resume 失败后，pending 仍未被破坏（state/version 不变 ⇒ 仍可消费）。
    let after_premature = pending_asks(&control_plane, session_id);
    assert_eq!(
        after_premature.len(),
        1,
        "T4：提前 resume 失败不得吞掉 pending ask，实际 = {after_premature:?}"
    );
    assert_eq!(
        after_premature[0].get("state").and_then(Value::as_str),
        Some("pending"),
        "T4：提前 resume 失败后 pending 必须仍是 pending（CAS 未被破坏），实际 = {:?}",
        after_premature[0]
    );
    assert_eq!(
        json_u64(&after_premature[0], "version", "T4"),
        version,
        "T4：提前 resume 失败不得推进 CAS 版本"
    );

    // Act ②：用过期的 expected_version 作答 ⇒ 必须 Err。
    let stale = control_plane.answer_ask_in_session(session_id, &request_id, version + 1, json!("继续"));
    expect_err(
        stale,
        "T4：answer_ask 必须以当前版本为 CAS 真值，过期版本必须 fail-closed 拒绝",
    );

    // Assert ②：过期作答失败后 pending 仍原样可消费。
    let after_stale = pending_asks(&control_plane, session_id);
    assert_eq!(
        after_stale.len(),
        1,
        "T4：过期版本作答失败不得删除 pending ask，实际 = {after_stale:?}"
    );
    assert_eq!(
        after_stale[0].get("state").and_then(Value::as_str),
        Some("pending"),
        "T4：过期版本作答失败后 pending 必须仍是 pending，实际 = {:?}",
        after_stale[0]
    );

    // Act ③：用正确版本作答 ⇒ 必须成功（证明前两次 Err 未损伤 CAS）。
    let consumed = expect_ok(
        control_plane.answer_ask_in_session(session_id, &request_id, version, json!("继续")),
        "T4：两次 fail-closed 之后，正确的 answer_ask 仍必须成功（CAS 未被破坏）",
    );

    // Assert ③：消费结果 state == consumed。
    assert_eq!(
        consumed.get("state").and_then(Value::as_str),
        Some("consumed"),
        "T4：answer_ask 成功后该请求必须被 CAS 消费为 consumed，实际 = {consumed}"
    );
}

// ---------------------------------------------------------------------------
// T5：control_outcome_pending 绝不回灌 provider（INV-3 + INV-4）
// ---------------------------------------------------------------------------

#[test]
fn t5_control_outcome_pending_never_reaches_provider() {
    let _serial = serial_guard();

    // Arrange
    let server = MockProviderServer::start(vec![
        MockReply::AskToolCall,
        MockReply::Text(CONTINUATION_TEXT.to_string()),
    ]);
    let workspace = isolated_workspace("t5");
    let control_plane = control_plane_with_mock_provider(server.base_url.clone(), &workspace);
    let sink = RecordingSink::new();
    let session_id = "session-ask-e2e-t5";

    // Act ①：挂起一轮。
    let pending = run_turn_until_ask_suspension(
        &control_plane,
        &sink,
        session_id,
        "turn-ask-e2e-t5-1",
        "T5",
    );
    let request_id = json_str(&pending, "requestId", "T5");
    let version = json_u64(&pending, "version", "T5");
    let run_id = require_run_id(&pending, "T5");

    // Assert ①：挂起时 provider 只被请求 1 次 —— 挂起标记绝不触发 followup 回灌。
    assert!(
        server.wait_request_count(1),
        "T5 夹具自检：挂起轮必须真实打到本地回环 provider"
    );
    assert_eq!(
        server.request_count(),
        1,
        "T5：Ask 挂起后 provider 请求次数必须恰为 1（禁止把 control_outcome_pending 作为工具错误回灌），\
         实际请求体 = {:?}",
        (0..server.request_count())
            .filter_map(|index| server.request_body(index))
            .collect::<Vec<_>>()
    );

    // Act ②：作答 + 恢复 ask-wait，再跑续轮。
    expect_ok(
        control_plane.answer_ask_in_session(session_id, &request_id, version, json!("继续")),
        "T5：answer_ask 必须以当前版本 CAS 消费挂起请求",
    );
    expect_ok(
        control_plane.graph_resume_ask(&run_id, &request_id, version, json!("继续")),
        "T5：作答后 graph_resume_ask 必须把该 run 的 ask-wait 恢复为可续",
    );
    run_continuation_turn(
        &control_plane,
        &sink,
        session_id,
        "turn-ask-e2e-t5-2",
        "T5",
    );

    // Assert ②：续轮**恰好**新增一次 provider 请求（总计 2 次）。
    assert!(
        server.wait_request_count(2),
        "T5：续轮必须在 {WAIT_BUDGET_MS}ms 内打出第 2 次 provider 请求（一次性注入消费后才会请求模型）"
    );
    assert_eq!(
        server.request_count(),
        2,
        "T5：挂起 + 续轮总共只允许 2 次 provider 请求（多余请求说明挂起标记或重试被回灌），实际 = {}",
        server.request_count()
    );

    // Assert ③：第 2 次请求体必须带原 call_id 的终结 tool 结果，且不得含挂起标记字面串。
    let second = server
        .request_body(1)
        .expect("T5：第 2 次 provider 请求体必须被 mock 记录");
    assert!(
        second.contains(ASK_CALL_ID),
        "T5：续轮的第 2 次 provider 请求必须包含原 Ask 的 call_id `{ASK_CALL_ID}` 对应的终结 tool 结果，\
         实际请求体 = {second}"
    );
    assert!(
        !second.contains("control_outcome_pending"),
        "T5（INV-4）：`control_outcome_pending` 挂起标记**绝不**能作为普通工具错误进入下一轮 provider 请求，\
         实际请求体 = {second}"
    );
}

// ═════════════════════════════════════════════════════════════════════════════
// T14 / L3-P 注入队列语义（红相）：失败不丢答案 / 同会话多条队列 / ack 边界
// ═════════════════════════════════════════════════════════════════════════════
//
// 契约依据（Lead 2026-11-06 裁决）：会话注入的 **ack 边界 = 成功终态（completed / suspended）**。
// provider 调用失败 / 取消时注入必须仍在（下一次续轮能取走）；成功终态后才 ack，且只结算一次。
// 反例来源：`/tmp/ask-audit` 的 `inject-drop`（BLOCKING）与 `inject-overwrite`（队列覆盖）。

/// 无 provider 的控制面：prepare 阶段必然失败（无可用 provider），用于复现「失败轮」。
fn plane_without_provider(graph_path: &Path, workspace: &Path) -> HostControlPlane {
    let runtime = AgentRuntimeBuilder::new()
        .session_store(isolated_session_store())
        .workspace_root(workspace)
        .build();
    HostControlPlaneBuilder::new()
        .runtime(runtime)
        .graph_run_store(GraphRunStore::persistent(graph_path))
        .build()
}

/// 有 mock provider 但 graph store **落盘**的控制面（便于观察会话注入的进出）。
fn plane_with_mock_provider_and_graph_path(
    base_url: String,
    workspace: &Path,
    graph_path: &Path,
) -> HostControlPlane {
    let runtime = AgentRuntimeBuilder::new()
        .session_store(isolated_session_store())
        .provider_resolver(Box::new(StaticProviderResolver {
            selection: provider_selection(base_url),
        }))
        .workspace_root(workspace)
        .build();
    HostControlPlaneBuilder::new()
        .runtime(runtime)
        .graph_run_store(GraphRunStore::persistent(graph_path))
        .build()
}

/// 直接经真实受管派发路径抛出一个 Ask（不依赖图谱绑定），返回 request_id。
fn dispatch_probe_ask(control_plane: &HostControlPlane, session_id: &str, call_id: &str) -> String {
    use pony_agent_core::agent::dispatcher::DispatchContext;
    use pony_agent_core::agent::tool_runtime::{InvocationOrigin, ToolDispatchRequest};

    let outcome = control_plane.ask_dispatcher.dispatch_governed(
        ToolDispatchRequest {
            origin: InvocationOrigin::Model,
            descriptor_id: "builtin:ask_user".to_string(),
            call_id: call_id.to_string(),
            arguments: json!({
                "question": ASK_QUESTION,
                "options": ["是", "否"],
                "description": "T14 注入队列红相"
            }),
        },
        &DispatchContext {
            session_id: Some(session_id.to_string()),
            run_id: None,
            turn_id: Some(format!("turn-{call_id}")),
            host_control_available: true,
            ..Default::default()
        },
    );
    outcome
        .control_outcome
        .expect("T14：host-mediated ask 必须返回 control outcome")
        .request_id
}

/// 读取落盘 graph store 里的**会话注入队列**（`sessionAskInjections`）的 requestId 列表。
fn session_injection_ids(graph_path: &Path) -> Vec<String> {
    let raw = std::fs::read_to_string(graph_path).unwrap_or_else(|_| "{}".to_string());
    let parsed: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
    let Some(object) = parsed.get("sessionAskInjections").and_then(Value::as_object) else {
        return Vec::new();
    };
    object
        .values()
        .filter_map(|entry| entry.get("requestId").and_then(Value::as_str))
        .map(str::to_string)
        .collect()
}

/// T14 ①（BLOCKING，`inject-drop`）：续轮**失败**不得吞掉会话注入。
///
/// 现状缺陷：注入在 prepare / 模型调用**之前**被一次性 `take`（破坏性读），失败路径不回滚 ⇒
/// 答案永久丢失。契约：ack 边界是成功终态；失败/取消后注入必须仍在，下一次续轮仍能取走。
#[test]
fn t6_failed_continuation_turn_must_not_drop_the_session_injection() {
    let _serial = serial_guard();
    let workspace = isolated_workspace("t6-inject-drop");
    let graph_path = workspace.join("graph-runs.json");
    let control_plane = plane_without_provider(&graph_path, &workspace);
    let session_id = "session-ask-e2e-t6";

    // Arrange：派发 + 作答 ⇒ 会话注入入队（无 run 绑定 ⇒ 走 session 队列）。
    let request_id = dispatch_probe_ask(&control_plane, session_id, "call-t6");
    expect_ok(
        control_plane.answer_ask_in_session(session_id, &request_id, 1, json!("是")),
        "T14：作答必须成功（持久化 CAS）",
    );
    assert_eq!(
        session_injection_ids(&graph_path),
        vec![request_id.clone()],
        "T14 前置：作答后必须恰好一条会话注入入队，实际 = {:?}",
        session_injection_ids(&graph_path)
    );

    // Act ①：跑一轮（无 provider ⇒ 必然 turn:failed）。
    let sink = RecordingSink::new();
    control_plane.start_turn_stream(
        &sink,
        StartTurnStreamCommand {
            turn_id: "turn-t6-fail-1".to_string(),
            input: test_turn_input(session_id, "继续"),
        },
    );
    let names = sink.names();
    assert!(
        names.iter().any(|name| name == "turn:failed"),
        "T14 前置：无 provider 时本轮必须以 turn:failed 收口，实际事件序列 = {names:?}"
    );

    // Assert（BLOCKING）：失败轮之后注入必须仍在（不得因破坏性 take 丢失）。
    assert_eq!(
        session_injection_ids(&graph_path),
        vec![request_id.clone()],
        "T14：失败/取消路径不得吞掉会话注入（take 早于 provider 且失败不回滚）；\
         契约要求 ack 边界=成功终态，失败后答案必须仍可被下一次续轮取走"
    );

    // Act ②：再失败一次。
    let sink_again = RecordingSink::new();
    control_plane.start_turn_stream(
        &sink_again,
        StartTurnStreamCommand {
            turn_id: "turn-t6-fail-2".to_string(),
            input: test_turn_input(session_id, "继续"),
        },
    );

    // Assert：多次失败仍**恰好一条**注入（不重复入队、不丢失 ⇒ 只结算一次）。
    assert_eq!(
        session_injection_ids(&graph_path),
        vec![request_id.clone()],
        "T14：连续失败轮不得重复入队也不得丢失，必须稳定保持恰好一条待结算注入"
    );

    // 持久化面：记录仍为 Answered（可据此重建注入，不依赖进程内存）。
    assert!(
        pending_asks(&control_plane, session_id).is_empty(),
        "T14：已作答记录不得回到待决列表"
    );
}

/// T14 ②（`inject-overwrite`）：同会话**两条**已作答 Ask 必须都在注入队列里（队列语义）。
///
/// 现状缺陷：`session_ask_injections: HashMap<session_id, injection>` 单槽 ⇒ 第二条覆盖第一条，
/// 第一条答案永久丢失。
#[test]
fn t7_two_answered_asks_in_one_session_must_both_stay_consumable() {
    let _serial = serial_guard();
    let workspace = isolated_workspace("t7-inject-overwrite");
    let graph_path = workspace.join("graph-runs.json");
    let control_plane = plane_without_provider(&graph_path, &workspace);
    let session_id = "session-ask-e2e-t7";

    let first = dispatch_probe_ask(&control_plane, session_id, "call-t7-1");
    let second = dispatch_probe_ask(&control_plane, session_id, "call-t7-2");
    expect_ok(
        control_plane.answer_ask_in_session(session_id, &first, 1, json!("第一条答案")),
        "T14：第一条作答必须成功",
    );
    expect_ok(
        control_plane.answer_ask_in_session(session_id, &second, 1, json!("第二条答案")),
        "T14：第二条作答必须成功",
    );

    let ids = session_injection_ids(&graph_path);
    assert!(
        ids.contains(&first) && ids.contains(&second),
        "T14：同会话两条已作答 Ask 必须都能被消费（第二条不得覆盖第一条）；\
         实际队列 = {ids:?}，缺失 = {:?}",
        [&first, &second]
            .into_iter()
            .filter(|id| !ids.contains(*id))
            .collect::<Vec<_>>()
    );
}

/// T14 ④（ack 边界对照组）：**成功终态**才 ack，且只结算一次（不得残留导致重复注入）。
///
/// 该条是既有正确面的冻结钉子（成功路径已实现），与 t6 的失败路径构成 ack 边界的两侧。
#[test]
fn t9_successful_turn_acks_the_session_injection_exactly_once() {
    let _serial = serial_guard();
    let server = MockProviderServer::start(vec![
        MockReply::Text("续轮完成：已按回答继续。".to_string()),
        MockReply::Text("第二轮正文".to_string()),
    ]);
    let workspace = isolated_workspace("t9-ack");
    let graph_path = workspace.join("graph-runs.json");
    let control_plane =
        plane_with_mock_provider_and_graph_path(server.base_url.clone(), &workspace, &graph_path);
    let session_id = "session-ask-e2e-t9";

    let request_id = dispatch_probe_ask(&control_plane, session_id, "call-t9");
    expect_ok(
        control_plane.answer_ask_in_session(session_id, &request_id, 1, json!("是")),
        "T14：作答必须成功",
    );
    assert_eq!(
        session_injection_ids(&graph_path),
        vec![request_id.clone()],
        "T14 前置：作答后必须入队"
    );

    // Act ①：成功轮。
    let sink = RecordingSink::new();
    control_plane.start_turn_stream(
        &sink,
        StartTurnStreamCommand {
            turn_id: "turn-t9-ok-1".to_string(),
            input: test_turn_input(session_id, "继续"),
        },
    );
    let names = sink.names();
    assert!(
        names.iter().any(|name| name == "turn:completed"),
        "T14 前置：有 provider 时本轮必须成功收口，实际事件序列 = {names:?}"
    );
    assert!(server.wait_request_count(1), "T14 前置：provider 必须收到请求");

    // Assert：成功终态 ⇒ ack，注入不得残留。
    assert!(
        session_injection_ids(&graph_path).is_empty(),
        "T14：成功终态后会话注入必须被 ack 清空（否则会在后续每一轮重复注入同一个答案）"
    );

    // Act ②：再跑一轮（不得再次注入）。
    let sink_again = RecordingSink::new();
    control_plane.start_turn_stream(
        &sink_again,
        StartTurnStreamCommand {
            turn_id: "turn-t9-ok-2".to_string(),
            input: test_turn_input(session_id, "再来一轮"),
        },
    );
    assert!(
        session_injection_ids(&graph_path).is_empty(),
        "T14：已 ack 的注入不得复活（只结算一次）"
    );
}

/// T14 ⑤（多进程文件竞态，`store-race`）：**暂不修复**，按 Lead 裁决标 `#[ignore]` 并登记。
///
/// 反例：两个 `GraphRunStore::persistent` 句柄各自持内存快照，先后写入同一 JSON 文件 ⇒ 后写
/// 覆盖前写，另一会话的注入丢失（`/tmp/ask-audit` `store-race`：after_second_process 只剩 B）。
/// 归因：这是 `GraphRunStore` 的**既有多进程文件竞态**（与 ask_persistence 的 CAS 面正交），
/// 修复需跨进程文件锁 / 单写者模型，超出 T14「注入队列语义」范围；已报 Lead 待单独立项。
#[test]
#[ignore = "既有多进程 GraphRunStore 文件竞态（后写覆盖前写），需跨进程锁/单写者模型，超出 T14 范围，已报 Lead"]
fn t8_two_persistent_graph_store_handles_must_not_lose_each_others_session_injections() {
    let _serial = serial_guard();
    let workspace = isolated_workspace("t8-store-race");
    let graph_path = workspace.join("graph-runs.json");

    let mut first_process = GraphRunStore::persistent(&graph_path);
    let mut second_process = GraphRunStore::persistent(&graph_path);

    first_process.queue_ask_injection_for_session(
        "session-A",
        pony_agent_core::agent::graph::GraphAskResumeInjection {
            request_id: "req-A".to_string(),
            run_id: "run-A".to_string(),
            turn_id: "turn-A".to_string(),
            call_id: "call-A".to_string(),
            tool_name: "Ask".to_string(),
            assistant_transcript: json!({ "callId": "call-A" }),
            terminal_result: json!({ "answer": "A" }),
        },
    );
    second_process.queue_ask_injection_for_session(
        "session-B",
        pony_agent_core::agent::graph::GraphAskResumeInjection {
            request_id: "req-B".to_string(),
            run_id: "run-B".to_string(),
            turn_id: "turn-B".to_string(),
            call_id: "call-B".to_string(),
            tool_name: "Ask".to_string(),
            assistant_transcript: json!({ "callId": "call-B" }),
            terminal_result: json!({ "answer": "B" }),
        },
    );

    let reopened = GraphRunStore::persistent(&graph_path);
    assert!(
        reopened
            .peek_ask_injection_for_session("session-A")
            .is_some(),
        "T14：第二个句柄写入不得覆盖第一个句柄已落盘的会话注入（多进程文件竞态）"
    );
    assert!(
        reopened
            .peek_ask_injection_for_session("session-B")
            .is_some(),
        "T14：两个会话的注入必须都能在重开后读到"
    );
}
