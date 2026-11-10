//! wave-65 审批（`Approval`）挂起可恢复 —— 黑盒端到端验收（独立 Test Agent 红相，B 级）。
//!
//! 契约真值源：`.dev-team/contract-matrix-approval-resume.md` §2 C5 + §1 INV-1..INV-5（frozen）。
//! 本文件是**唯一**写域：不引用任何 `cfg(test)` 私有夹具，全部经 public 入口驱动
//! （`AgentRuntimeBuilder` / `HostControlPlaneBuilder` / `HostControlPlane` 的宿主命令面）。
//!
//! 覆盖与判据（判据逐字来自 C5）：
//! - **T1**（C1+C3）：宿主打开 `host_approval_resume(true)` 后，模型对**工作区外**路径发起
//!   `Read` ⇒ 本轮以 `turn:suspended` 收口（恰 1 条）、`turn:failed` 恰 0 条；
//!   `list_pending_approvals(session)` 恰 1 条且带 `requestId` / `version` / `options.path`
//!   == 该越界路径；挂起轮的 provider 请求体**不含** `control_outcome_pending`
//!   （挂起标记绝不作为普通工具错误回灌）。
//! - **T2**（C3+C4）：`authorize_path(path,"read")` + `approve_control_request(request_id,
//!   version)` 后用固定宿主续轮文案再跑一轮 ⇒ 同一 `Read` 被放行，续轮 `turn:completed` 恰 1 条，
//!   不新增 `turn:suspended` / `turn:failed`，续轮 provider 请求体不含 `control_outcome_pending`；
//!   批准后 `list_pending_approvals` 为空。
//! - **T3**（INV-4）：`cancel_approval_request(request_id, version)` 必须 `Ok`；随后
//!   `approve_control_request(request_id, version)` 必须 `Err`（CAS 未放松），拒绝后请求不在待决列表。
//! - **T4**（INV-2）：同一夹具但**不**调用 `host_approval_resume`（默认 `false`）⇒
//!   `turn:failed` 恰 1 条且 error 以 `approval_resume_unavailable` 开头、`turn:suspended` 恰 0 条。
//!
//! 红相说明（写于实施前，如实记录当前基线）：C3（`list_pending_approvals` /
//! `cancel_approval_request`）当前是空桩（恒空数组 / 恒 `Err("not_implemented")`），C2 的审批
//! 结构化 `options` 与 C4 的批准闭环尚未实现，故 **T1/T2/T3 现在必然失败**（失败均来自断言，
//! 不是编译错误），**T4 允许绿**（默认能力位下 fail-fast 语义逐字未变）。实施完成后本文件转绿
//! 且此后只读。
//!
//! 纪律（test-expert L2）：真实断言、AAA、命名带判据号；禁止 `#[ignore]` / catch 吞异常 /
//! 空断言；跨线程/异步等待一律显式条件轮询（`wait_until`）+ 硬上界，绝不裸 sleep 盲等。

use std::collections::VecDeque;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
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
    HostControlPlane, HostControlPlaneBuilder, StartTurnStreamCommand,
};
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntimeBuilder, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::SessionStore;
use pony_agent_core::agent::turn_flow::TurnEventSink;

/// mock provider 第 1 跳里 `Read` 工具调用的 id。
const READ_CALL_ID: &str = "call-approval-e2e-1";
/// mock provider 最后一段 assistant 文本（T2 断言 `turn:completed.text` 与之逐字相等）。
const CONTINUATION_TEXT: &str = "续轮完成：已按批准结果继续执行。";
/// C4 规定的固定宿主续轮文案（非空，否则 `prepare_turn` 的 `reject_empty` 会拒绝）。
const CONTINUATION_PROMPT: &str = "（已批准，继续上一轮未完成的步骤）";
/// 越界探针文件的正文（良性内容，T2 批准后会被真实读取，绝不使用敏感文件）。
const PROBE_CONTENT: &str = "approval-resume probe line\n";
/// 显式条件轮询的硬上界（毫秒）。
const WAIT_BUDGET_MS: u64 = 10_000;

/// 集成测试（`tests/*.rs`）编译 crate 时不启用 `cfg(test)`，`SessionStore::new()` 会走生产分支
/// 指向用户真实数据目录。这里在**任何**控制面/运行时构造之前（`Once` 保证顺序）把数据目录基准
/// 重定向到本测试二进制独有的临时沙箱，确保集成测试绝不读写生产数据（与
/// `ask_resume_e2e.rs` 同法）。
static ISOLATE_PROCESS_DATA_DIR: Once = Once::new();

fn isolate_process_data_dir() {
    ISOLATE_PROCESS_DATA_DIR.call_once(|| {
        let unique = format!(
            "pony-agent-it-approval-resume-{}-{}",
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

/// 每个用例独占的工作区根（位于系统临时目录下，与越界探针文件**同父不同子**）。
fn isolated_workspace(test_name: &str) -> PathBuf {
    isolate_process_data_dir();
    let dir = std::env::temp_dir().join(format!(
        "pony-agent-approval-resume-ws-{}-{}",
        std::process::id(),
        test_name
    ));
    std::fs::create_dir_all(&dir).expect("create isolated workspace dir");
    dir
}

/// 工作区**之外**的真实文件（良性内容）：工作区外 `Read` 因此命中「需要显式授权」路径，
/// 又不触碰任何敏感内容。目录与工作区根同级但不同子树 ⇒ 必然在 root 之外。
fn outside_workspace_probe_file(test_name: &str) -> PathBuf {
    isolate_process_data_dir();
    let dir = std::env::temp_dir().join(format!(
        "pony-agent-approval-resume-outside-{}-{}",
        std::process::id(),
        test_name
    ));
    std::fs::create_dir_all(&dir).expect("create outside workspace dir");
    let file = dir.join("probe.txt");
    std::fs::write(&file, PROBE_CONTENT).expect("write outside workspace probe file");
    file
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

/// 用例串行闸门（防 Flaky，非断言）：同进程用例共享同一沙箱 `sessions.db`，并行跑时会互相
/// 触发 `database is locked`。这里只串行**夹具**（不含任何判定），断言强度不变。
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
    /// 非流式决策：一次 `Read` 工具调用（`path` 指向工作区外文件）。
    ReadToolCall(String),
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
                    Some(MockReply::ReadToolCall(path)) => read_tool_call_body(READ_CALL_ID, &path),
                    Some(MockReply::Text(text)) => text_completion_body(&text),
                    // 队列耗尽：返回一段可辨识文本，绝不 hang / 绝不伪造 tool_call，
                    // 让多余请求在断言里显形。
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

    fn bodies_from(&self, index: usize) -> Vec<String> {
        let requests = self
            .requests
            .lock()
            .expect("mock provider request lock poisoned");
        requests.iter().skip(index).cloned().collect()
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

/// 决策段应答：assistant 发起一次 `Read` 工具调用，`path` 指向工作区**之外**的真实文件
/// （因此治理面命中「需要显式授权」，工具本身不会在批准前执行）。
fn read_tool_call_body(call_id: &str, path: &str) -> String {
    let arguments = json!({
        "path": path,
        "description": "读取工作区外的探针文件",
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
                        "name": "Read",
                        "arguments": arguments,
                    }
                }]
            }
        }],
        "usage": { "prompt_tokens": 5, "completion_tokens": 5, "total_tokens": 10 }
    })
    .to_string()
}

/// 完结段应答：assistant 纯文本（续轮的真实产出）。
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
        requested_name: "approval-resume-test".to_string(),
        provider_name: "approval-resume-test".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "APPROVAL_RESUME_TEST_API_KEY".to_string(),
        api_key: Some("approval-resume-test-key".to_string()),
        model: "approval-resume-test-model".to_string(),
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

/// T1~T3 的夹具：**TUI 宿主形态**建栈 —— `AgentRuntimeBuilder::desktop()` +
/// `host_approval_resume(true)`（C1 唯一消费点）+ 隔离会话库 + 独立 workspace root +
/// 内存 graph run store（`AgentRuntimeBuilder` 默认装配 governed executor，审批治理链路依赖它）。
fn control_plane_with_approval_resume(base_url: String, workspace: &Path) -> HostControlPlane {
    let runtime = AgentRuntimeBuilder::desktop()
        .host_approval_resume(true)
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

/// T4（INV-2）的夹具：**不**调用 `host_approval_resume`，即默认 `false`（桌面 / CLI 形态）。
fn control_plane_with_default_runtime(base_url: String, workspace: &Path) -> HostControlPlane {
    let runtime = AgentRuntimeBuilder::desktop()
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

// ---------------------------------------------------------------------------
// 断言辅助（JSON 投影读取 + 带判据号的消息）
// ---------------------------------------------------------------------------

/// 该会话作用域下的待决审批快照（C3：`list_pending_approvals(session_id)`）。
fn pending_approvals(control_plane: &HostControlPlane, session_id: &str) -> Vec<Value> {
    match control_plane.list_pending_approvals(session_id) {
        Value::Array(items) => items,
        other => panic!("list_pending_approvals 必须返回 JSON 数组，实际 = {other}"),
    }
}

fn json_str(request: &Value, key: &str, case: &str) -> String {
    request
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{case}：审批快照必须带字符串字段 `{key}`，实际 = {request}"))
        .to_string()
}

fn json_u64(request: &Value, key: &str, case: &str) -> u64 {
    request
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{case}：审批快照必须带无符号整数字段 `{key}`，实际 = {request}"))
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

/// 夹具：经宿主真实流式轮入口（`start_ask_aware_turn_stream`，TUI 同款 graph run 路径）
/// 跑一轮。
fn run_turn(
    control_plane: &HostControlPlane,
    sink: &RecordingSink,
    session_id: &str,
    turn_id: &str,
    message: &str,
    case: &str,
) {
    expect_ok(
        control_plane.start_ask_aware_turn_stream(
            sink,
            StartTurnStreamCommand {
                turn_id: turn_id.to_string(),
                input: test_turn_input(session_id, message),
            },
        ),
        case,
    );
}

/// 夹具：跑一轮直到「审批挂起」，返回该会话的待决审批快照（C3 的唯一读取面）。
fn run_turn_until_approval_suspension(
    control_plane: &HostControlPlane,
    sink: &RecordingSink,
    session_id: &str,
    turn_id: &str,
    case: &str,
) -> Value {
    run_turn(
        control_plane,
        sink,
        session_id,
        turn_id,
        "请读取工作区外的探针文件，然后继续",
        case,
    );

    let names = sink.names();
    assert!(
        names.iter().any(|name| name == "turn:suspended"),
        "{case}：工作区外 Read 必须让本轮以 turn:suspended 收口（审批挂起可恢复），实际事件序列 = {names:?}"
    );

    let approvals = pending_approvals(control_plane, session_id);
    assert_eq!(
        approvals.len(),
        1,
        "{case}：挂起后 list_pending_approvals(session) 必须恰有一条待决审批，实际 = {approvals:?}"
    );
    approvals[0].clone()
}

// ---------------------------------------------------------------------------
// T1：能力开 + 工作区外 Read ⇒ 可恢复挂起 + 宿主可列出待决审批（C1 + C3）
// ---------------------------------------------------------------------------

#[test]
fn t1_host_approval_resume_suspends_outside_workspace_read_and_lists_the_pending_approval() {
    let _serial = serial_guard();

    // Arrange：mock 第 1 跳返回越界路径的 Read；宿主打开审批恢复能力位。
    let outside = outside_workspace_probe_file("t1");
    let outside_path = outside.display().to_string();
    let server = MockProviderServer::start(vec![MockReply::ReadToolCall(outside_path.clone())]);
    let workspace = isolated_workspace("t1");
    let control_plane = control_plane_with_approval_resume(server.base_url.clone(), &workspace);
    let sink = RecordingSink::new();
    let session_id = "session-approval-e2e-t1";

    // Act
    let approval = run_turn_until_approval_suspension(
        &control_plane,
        &sink,
        session_id,
        "turn-approval-e2e-t1-1",
        "T1",
    );

    // Assert ①：事件语义 —— suspended 恰 1 条、failed 恰 0 条（可恢复挂起不是失败）。
    assert_eq!(
        sink.events_named("turn:suspended").len(),
        1,
        "T1（C1）：审批能力位打开时，工作区外 Read 必须派发**恰好一条** turn:suspended，实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        sink.events_named("turn:failed").len(),
        0,
        "T1（C1）：可恢复的审批挂起**禁止**派生 turn:failed，实际事件序列 = {:?}",
        sink.names()
    );

    // Assert ②：宿主投影 —— requestId / version 存在，requestKind 为 approval，options.path 正确。
    assert!(
        !json_str(&approval, "requestId", "T1").trim().is_empty(),
        "T1（C3）：待决审批必须暴露非空 requestId，实际 = {approval}"
    );
    assert!(
        json_u64(&approval, "version", "T1") >= 1,
        "T1（C3）：待决审批必须暴露 CAS 起始版本（>= 1），实际 = {approval}"
    );
    assert_eq!(
        approval.get("requestKind").and_then(Value::as_str),
        Some("approval"),
        "T1（C3）：待决审批的 requestKind 必须投影为 `approval`，实际 = {approval}"
    );
    assert_eq!(
        approval.get("sessionId").and_then(Value::as_str),
        Some(session_id),
        "T1（C3）：待决审批必须绑定发起挂起的会话，实际 = {approval}"
    );
    let options = approval
        .get("options")
        .unwrap_or_else(|| panic!("T1（C2/C3）：待决审批必须带结构化 options，实际 = {approval}"));
    assert_eq!(
        options.get("path").and_then(Value::as_str),
        Some(outside_path.as_str()),
        "T1（C2/C3）：options.path 必须是模型给出的**原始**越界路径 `{outside_path}`，实际 = {approval}"
    );
    assert!(
        approval.get("nonce").is_none(),
        "T1（INV-3）：宿主投影**不得**包含 nonce（CAS 四元组由宿主内部持有），实际 = {approval}"
    );

    // Assert ③：挂起绝不回灌 provider —— 只被请求挂起前那一次，且请求体无挂起标记字面串。
    assert!(
        server.wait_request_count(1),
        "T1 夹具自检：挂起轮必须真实打到本地回环 provider"
    );
    assert_eq!(
        server.request_count(),
        1,
        "T1（C5）：审批挂起后 provider 请求次数必须恰为 1（不得把 control_outcome_pending 作为工具错误回灌），实际请求体 = {:?}",
        server.bodies_from(0)
    );
    let first = server
        .request_body(0)
        .expect("T1：第 1 次 provider 请求体必须被 mock 记录");
    assert!(
        !first.contains("control_outcome_pending"),
        "T1（C5）：挂起轮的 provider 请求体**不得**包含 `control_outcome_pending` 字面串，实际 = {first}"
    );

    // 工具在批准前绝不执行（INV-1）：探针文件仍原封不动，且本轮从未产出 completed。
    assert_eq!(
        sink.events_named("turn:completed").len(),
        0,
        "T1（INV-1）：未获批准前本轮不得完成（工具不得执行），实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        std::fs::read_to_string(&outside).expect("T1：探针文件必须仍可读"),
        PROBE_CONTENT,
        "T1（INV-1）：批准前工具不得执行，探针文件内容必须保持不变"
    );
}

// ---------------------------------------------------------------------------
// T2：authorize_path + approve_control_request 后续轮放行同一 Read（C3 + C4）
// ---------------------------------------------------------------------------

#[test]
fn t2_approval_then_authorized_continuation_turn_completes_the_read() {
    let _serial = serial_guard();

    // Arrange：mock 队列 = [Read(挂起), Read(已批准), 文本(完结)]。
    let outside = outside_workspace_probe_file("t2");
    let outside_path = outside.display().to_string();
    let server = MockProviderServer::start(vec![
        MockReply::ReadToolCall(outside_path.clone()),
        MockReply::ReadToolCall(outside_path.clone()),
        MockReply::Text(CONTINUATION_TEXT.to_string()),
    ]);
    let workspace = isolated_workspace("t2");
    let control_plane = control_plane_with_approval_resume(server.base_url.clone(), &workspace);
    let sink = RecordingSink::new();
    let session_id = "session-approval-e2e-t2";

    let approval = run_turn_until_approval_suspension(
        &control_plane,
        &sink,
        session_id,
        "turn-approval-e2e-t2-1",
        "T2",
    );
    let request_id = json_str(&approval, "requestId", "T2");
    let version = json_u64(&approval, "version", "T2");
    assert!(
        server.wait_request_count(1),
        "T2 夹具自检：挂起轮必须真实打到本地回环 provider"
    );
    assert_eq!(
        server.request_count(),
        1,
        "T2 夹具自检：挂起轮只应产生一次 provider 请求，实际请求体 = {:?}",
        server.bodies_from(0)
    );

    // Act ①：TUI 批准链路的两步 —— 先写 read 作用域授权，再 CAS 消费审批请求。
    expect_ok(
        control_plane.authorize_path(&outside_path, "read"),
        "T2（C4）：批准前必须能以 read 作用域授权该越界路径",
    );
    expect_ok(
        control_plane.approve_control_request(&request_id, version),
        "T2（C3/C4）：approve_control_request 必须以当前版本 CAS 消费待决审批",
    );

    // Assert ①：批准后待决列表清空（该请求不再可被重复批准）。
    assert!(
        pending_approvals(&control_plane, session_id).is_empty(),
        "T2（C3）：批准后 list_pending_approvals 必须为空，实际 = {:?}",
        pending_approvals(&control_plane, session_id)
    );

    // Act ②：固定宿主续轮文案跑续轮。
    run_turn(
        &control_plane,
        &sink,
        session_id,
        "turn-approval-e2e-t2-2",
        CONTINUATION_PROMPT,
        "T2：续轮入口必须成功返回（不得把可续轮当成错误上抛）",
    );

    // Assert ②：续轮产出**恰好一条** turn:completed，正文 == mock 最后一段文本。
    let completed = sink.events_named("turn:completed");
    assert_eq!(
        completed.len(),
        1,
        "T2（C5）：续轮必须产出**恰好一条** turn:completed，实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        completed[0].text.as_deref(),
        Some(CONTINUATION_TEXT),
        "T2（C5）：turn:completed 的正文必须等于 mock 最后一段 assistant 文本（同一 Read 被放行后模型真的继续了）"
    );

    // Assert ③：不新增 suspended / failed（挂起轮那 1 条 suspended 保留，failed 始终 0）。
    assert_eq!(
        sink.events_named("turn:suspended").len(),
        1,
        "T2（C5）：批准后续轮**不得**新增 turn:suspended（总数仍应为挂起轮那 1 条），实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        sink.events_named("turn:failed").len(),
        0,
        "T2（C5）：批准后续轮**不得**派生 turn:failed，实际事件序列 = {:?}",
        sink.names()
    );

    // Assert ④：续轮真的打到 provider，且续轮的所有请求体都不含挂起标记字面串、
    // 且至少有一次重新发起了同一越界 Read（放行后工具链路真的继续）。
    assert!(
        server.wait_request_count(2),
        "T2：续轮必须在 {WAIT_BUDGET_MS}ms 内打出第 2 次 provider 请求"
    );
    let continuation_bodies = server.bodies_from(1);
    assert!(
        !continuation_bodies.is_empty(),
        "T2：续轮必须留下 provider 请求体（否则无法判定回灌语义）"
    );
    for (offset, body) in continuation_bodies.iter().enumerate() {
        assert!(
            !body.contains("control_outcome_pending"),
            "T2（C5/INV-4）：续轮第 {} 次 provider 请求体**不得**包含 `control_outcome_pending`，实际 = {body}",
            offset + 2
        );
    }
    // 比对标识用「探针文件所在目录名」而不是整条路径：目录名只含 ASCII 且**不含路径分隔符**，
    // 因此不受 JSON 转义层级影响（Windows 的 `C:\a\b` 在请求体里可能是 `\\` 甚至 `\\\\`；
    // 此前按原始路径比对在 Windows CI 恒假，Linux 无反斜杠故未暴露）。目录名带 pid + 用例名，
    // 唯一性足够。
    let outside_dir_marker = outside
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .expect("outside workspace dir name")
        .to_string();
    assert!(
        continuation_bodies
            .iter()
            .any(|body| body.contains(&outside_dir_marker)),
        "T2（C5）：续轮必须重新发起同一越界 Read（请求体必须包含 `{outside_dir_marker}`，路径 `{outside_path}`），实际 = {continuation_bodies:?}"
    );
}

// ---------------------------------------------------------------------------
// T3：拒绝（cancel）终态 + CAS 不放松（INV-4）
// ---------------------------------------------------------------------------

#[test]
fn t3_cancel_approval_request_is_terminal_and_approve_after_cancel_fails_closed() {
    let _serial = serial_guard();

    // Arrange：先真实挂起一条审批请求。
    let outside = outside_workspace_probe_file("t3");
    let outside_path = outside.display().to_string();
    let server = MockProviderServer::start(vec![MockReply::ReadToolCall(outside_path)]);
    let workspace = isolated_workspace("t3");
    let control_plane = control_plane_with_approval_resume(server.base_url.clone(), &workspace);
    let sink = RecordingSink::new();
    let session_id = "session-approval-e2e-t3";
    let approval = run_turn_until_approval_suspension(
        &control_plane,
        &sink,
        session_id,
        "turn-approval-e2e-t3-1",
        "T3",
    );
    let request_id = json_str(&approval, "requestId", "T3");
    let version = json_u64(&approval, "version", "T3");

    // Act ①：拒绝（取消）该审批请求 —— C3 的宿主拒绝面。
    let cancelled = expect_ok(
        control_plane.cancel_approval_request(&request_id, version),
        "T3（C3）：cancel_approval_request 必须以当前版本成功取消待决审批",
    );
    assert!(
        cancelled.to_string().contains(&request_id),
        "T3（C3）：取消结果必须回带被取消的 requestId `{request_id}`，实际 = {cancelled}"
    );

    // Act ②：同一版本再次批准 ⇒ 必须 Err（CAS 未破，INV-4）。
    let error = expect_err(
        control_plane.approve_control_request(&request_id, version),
        "T3（INV-4）：已被拒绝的审批不得被同版本再次批准（CAS 不得放松）",
    );
    assert!(
        !error.trim().is_empty(),
        "T3（INV-4）：拒绝后的批准失败必须携带可读原因，实际 = {error:?}"
    );

    // Assert ①：拒绝后请求不在待决列表。
    assert!(
        pending_approvals(&control_plane, session_id).is_empty(),
        "T3（C5）：拒绝后该审批请求不得停留在待决列表，实际 = {:?}",
        pending_approvals(&control_plane, session_id)
    );

    // Assert ②：拒绝轮本身仍是一次可恢复挂起（不是失败），且工具未被放行执行。
    assert_eq!(
        sink.events_named("turn:failed").len(),
        0,
        "T3：拒绝链路不得把挂起轮改写成 turn:failed，实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        sink.events_named("turn:completed").len(),
        0,
        "T3（INV-1）：拒绝后工具不得执行，本轮不得产出 turn:completed，实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        std::fs::read_to_string(&outside).expect("T3：探针文件必须仍可读"),
        PROBE_CONTENT,
        "T3（INV-1）：拒绝路径不得改动/执行探针文件"
    );
}

// ---------------------------------------------------------------------------
// T4：INV-2 回归 —— 默认能力位（false）仍 fail-fast
// ---------------------------------------------------------------------------

#[test]
fn t4_default_runtime_fails_closed_with_approval_resume_unavailable() {
    let _serial = serial_guard();

    // Arrange：同一夹具，但**不**调用 host_approval_resume（默认 false ⇒ 桌面/CLI 形态）。
    let outside = outside_workspace_probe_file("t4");
    let outside_path = outside.display().to_string();
    let server = MockProviderServer::start(vec![MockReply::ReadToolCall(outside_path)]);
    let workspace = isolated_workspace("t4");
    let control_plane = control_plane_with_default_runtime(server.base_url.clone(), &workspace);
    let sink = RecordingSink::new();
    let session_id = "session-approval-e2e-t4";

    // Act
    run_turn(
        &control_plane,
        &sink,
        session_id,
        "turn-approval-e2e-t4-1",
        "请读取工作区外的探针文件",
        "T4：默认能力位下本轮入口仍须正常返回（失败以终态事件表达）",
    );

    // Assert ①：终态必须是 turn:failed（恰一条）。
    let failed = sink.events_named("turn:failed");
    assert_eq!(
        failed.len(),
        1,
        "T4（INV-2）：能力位关闭时审批挂起必须 fail-closed 收口为**恰好一条** turn:failed，实际事件序列 = {:?}",
        sink.names()
    );

    // Assert ②：error 必须以 approval_resume_unavailable 开头（机器可判别的错误码）。
    let error = failed[0].error.as_deref().unwrap_or_default();
    assert!(
        error.starts_with("approval_resume_unavailable"),
        "T4（INV-2）：失败终态的 error 必须以 `approval_resume_unavailable` 开头，实际 = {error:?}"
    );

    // Assert ③：不得出现无终态 suspended。
    assert_eq!(
        sink.events_named("turn:suspended").len(),
        0,
        "T4（INV-2）：不可恢复的审批挂起**绝不**允许下发无终态 turn:suspended，实际事件序列 = {:?}",
        sink.names()
    );

    // Assert ④：工具在失败收口下同样不得执行（INV-1）。
    assert_eq!(
        sink.events_named("turn:completed").len(),
        0,
        "T4（INV-1）：fail-closed 收口下不得产出 turn:completed，实际事件序列 = {:?}",
        sink.names()
    );
    assert_eq!(
        std::fs::read_to_string(&outside).expect("T4：探针文件必须仍可读"),
        PROBE_CONTENT,
        "T4（INV-1）：fail-closed 路径不得执行工具改动探针文件"
    );
}
