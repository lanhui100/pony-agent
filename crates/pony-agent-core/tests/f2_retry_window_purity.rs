//! [diag-tps / task-5] F2「重试取样窗口纯净」验收（独立测试者 L2-T，红相先写、冻结）。
//!
//! 契约真值源（Lead 冻结，只读）：共享任务板 `task-5` 的 I1–I4 + 诊断综合
//! `.dev-team/diag-tps/00-SYNTHESIS.md` §3 缺陷 2（「per-call 的 dur/ttft 取样点在重试循环之外，
//! 失败尝试（429/525，1.9–8.0s）被计进窗口」）。
//!
//! 判据口径（只锁可观察行为，不锁实现细节）：
//! - 用**本地回环 mock provider**（`127.0.0.1:0`，零外网）构造「第一次 attempt 失败耗时 X、
//!   第二次成功」的真实调用链；失败面取 **503**（既有 `TransientRetryable` 语义 ⇒ 仓库当前
//!   确实会重试，反向锁住「不得靠禁掉重试来通过本卡」）。
//! - 断言落在**终态事件的 per-call 记录**（`turn:completed.providerCallRecords[0]`）上：
//!   记录里的生成窗口必须只覆盖**最后一次成功 attempt**，不得包含 X。
//! - I2 的「attempts 次数作为独立字段记录」：在终态 payload 的 JSON 里按**语义**取
//!   attempts 相关数值（键名任意嵌套、任意前后缀，只要末段含 `attempt` 且取值为 2）。
//!
//! 为什么用 buffered（`supports_streaming=false`）链路构造：
//! 该形态下 `first_token_latency_ms` 天然缺席 ⇒ 生成窗口 `gen = dur`，失败尝试耗时
//! **无处可躲**地被算进分母（这正是诊断裁决里 end_to_end 被压到 10.4 t/s 的机制）。
//! 流式链路里 `gen = dur - ttft` 会部分自相抵消，反而掩盖缺陷。
//!
//! 纪律（test-expert L2）：
//! - AAA 结构；mock 剧本固定、无随机、无「盲等」sleep（失败响应的 X 是**被测输入**，
//!   不是等待同步）；无 `#[ignore]`；每条用例均有非空断言。
//! - 红相锚定：当前基线把 700ms 失败 attempt + ~500ms 退避睡眠都算进 `turn_duration_ms`
//!   ⇒ `f2_*` 的窗口上界断言与 attempts 断言必失败（断言失败，非编译错误）。
//!
//! 本文件由 Test Agent 独占写域。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::context::DefaultTurnContextBuilder;
use pony_agent_core::agent::otel::log::{set_log_file_path};
use pony_agent_core::agent::planner::LocalTurnPlanner;
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntime, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::{FileSessionBackend, SessionStore};
use pony_agent_core::agent::telemetry::{
    DefaultTurnTelemetryBuilder, ProviderCallCacheRecord,
};
use pony_agent_core::agent::tools::{ToolCall, ToolExecutor, ToolResult};
use pony_agent_core::agent::turn_flow::TurnEventSink;
use serde_json::Value;

/// 失败 attempt 的**可辨识耗时** X：必须显著大于任何一次真实调用的耗时，
/// 又不足以拖慢测试（700ms 失败 + 一次退避）。
const FAILED_ATTEMPT_MS: u64 = 700;

/// 成功后一次调用的输出 token 数（速度口径的分子；用于反向印证分母干净）。
const SUCCESS_OUTPUT_TOKENS: u64 = 40;

/// 成功响应体（OpenAI chat/completions 200 形态 + usage）。
const OPENAI_OK_BODY: &str = "{\"id\":\"f2-ok\",\"choices\":[{\"index\":0,\"message\":{\"role\":\"assistant\",\"content\":\"f2-retry-window-ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":40,\"total_tokens\":140}}";

/// 失败响应体：503 + `server_error` 措辞（命中既有 `TransientRetryable`，保留重试语义）。
const SERVER_ERROR_BODY: &str =
    "{\"error\":{\"type\":\"server_error\",\"message\":\"upstream temporarily unavailable\"}}";

/// 生成窗口的**物理上界**：只覆盖最后一次成功 attempt 时，`gen` 只该是「一次本地回环
/// HTTP + JSON 解析」的量级。700ms 的失败 attempt 一旦被算进来，即刻越界。
const GENERATION_WINDOW_BOUND_MS: u64 = 350;

// ---------------------------------------------------------------------------
// 回环 mock provider（剧本应答，非流式）
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct MockReply {
    status: u16,
    reason: &'static str,
    body: &'static str,
    content_type: &'static str,
    /// 应答前的固定延迟 —— 模拟「失败 attempt 自身耗时 X」（**被测输入**，不是等待同步）。
    delay: Duration,
}

struct ScriptedProvider {
    base_url: String,
    addr: String,
    requests: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<Value>>>,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ScriptedProvider {
    fn script(replies: Vec<MockReply>) -> Self {
        assert!(!replies.is_empty(), "F2：mock 剧本不得为空");
        let listener = TcpListener::bind("127.0.0.1:0").expect("F2：mock provider 必须能绑定回环端口");
        let port = listener
            .local_addr()
            .expect("F2：mock provider 必须能取本地地址")
            .port();
        let requests = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let requests_for_thread = Arc::clone(&requests);
        let bodies_for_thread = Arc::clone(&bodies);
        let shutdown_for_thread = Arc::clone(&shutdown);

        let handle = std::thread::spawn(move || {
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _addr)) = listener.accept() else {
                    return;
                };
                let raw = read_http_request(&mut stream);
                if raw.is_empty() {
                    // 收尾唤醒连接，不计入真实请求。
                    continue;
                }
                if let Ok(json) = serde_json::from_slice::<Value>(&raw) {
                    bodies_for_thread
                        .lock()
                        .expect("F2：mock bodies 锁被毒化")
                        .push(json);
                }
                let index = requests_for_thread.fetch_add(1, Ordering::SeqCst);
                let reply = replies.get(index).copied().unwrap_or_else(|| {
                    *replies.last().expect("F2：剧本非空")
                });
                if !reply.delay.is_zero() {
                    std::thread::sleep(reply.delay);
                }
                let head = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    reply.status,
                    reply.reason,
                    reply.content_type,
                    reply.body.as_bytes().len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(reply.body.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                drain_incoming(&mut stream);
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            requests,
            bodies,
            shutdown,
            handle: Some(handle),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    /// mock 侧真实收到的请求体（**证据面**：证明两次 attempt 都真的是 HTTP 请求）。
    fn received_bodies(&self) -> Vec<Value> {
        self.bodies
            .lock()
            .expect("F2：mock bodies 锁被毒化")
            .clone()
    }
}

impl Drop for ScriptedProvider {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // 用一次空连接唤醒 accept，令 mock 线程及时退出。
        let _ = TcpStream::connect(self.addr.clone());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut raw: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 4096];
    let mut expected_total: Option<usize> = None;
    loop {
        if let Some(total) = expected_total {
            if raw.len() >= total {
                break;
            }
        }
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                raw.extend_from_slice(&buffer[..read]);
                if expected_total.is_none() {
                    if let Some(header_end) = find_subslice(&raw, b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&raw[..header_end]).to_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        expected_total = Some(header_end + 4 + length);
                    }
                }
            }
            Err(_) => break,
        }
    }
    match find_subslice(&raw, b"\r\n\r\n") {
        Some(index) => raw[index + 4..].to_vec(),
        None => Vec::new(),
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn drain_incoming(stream: &mut TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
    let mut chunk = [0u8; 4096];
    while matches!(stream.read(&mut chunk), Ok(read) if read > 0) {}
}

// ---------------------------------------------------------------------------
// 取证载体：结构化日志落盘（F2 I2「attempts / 失败耗时作为独立字段记录」）
// ---------------------------------------------------------------------------
//
// 日志 sink（`otel::log`）是**进程级全局态**，两条用例必须串行，避免互相覆盖路径。
static F2_LOCK: Mutex<()> = Mutex::new(());

fn f2_lock() -> std::sync::MutexGuard<'static, ()> {
    F2_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn temp_log_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "pony-f2-retry-window-{}-{tag}.log",
        std::process::id()
    ))
}

/// 读取落盘日志并逐行解析 JSON（坏行/非 JSON 行跳过——stdout 面混入的 eprintln 不算证据）。
fn log_lines(path: &Path) -> Vec<Value> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line.trim()).ok())
        .collect()
}

/// 在一条日志/载荷（已展平）里按**语义**找 attempts 证据：末段键名含 `attempt` 且取值为期望次数。
fn attempts_evidence(flat: &[(String, Value)], expected: u64) -> Option<String> {
    flat.iter()
        .find(|(path, leaf)| {
            last_key(path).to_lowercase().contains("attempt")
                && leaf.as_u64() == Some(expected)
        })
        .map(|(path, _)| path.clone())
}

/// 在一条日志/载荷（已展平）里找「失败 attempt 自身耗时」证据（独立字段，且 > 0）。
fn failed_attempt_evidence(flat: &[(String, Value)]) -> Option<(String, u64)> {
    flat.iter()
        .find_map(|(path, leaf)| {
            let key = last_key(path).to_lowercase();
            let looks_like_failed_duration = key.contains("fail")
                && (key.contains("ms") || key.contains("duration"));
            match leaf.as_u64() {
                Some(number) if looks_like_failed_duration && number > 0 => {
                    Some((path.clone(), number))
                }
                _ => None,
            }
        })
}

// ---------------------------------------------------------------------------
// runtime 装配（与仓库既有 full-turn 验收同构）
// ---------------------------------------------------------------------------

struct StaticResolver {
    selection: ResolvedProviderSelection,
}

impl ProviderSelectionResolver for StaticResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        self.selection.clone()
    }
}

struct NoopExecutor;

impl ToolExecutor for NoopExecutor {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        ToolResult {
            tool_name: call.name.clone(),
            status: "ok".to_string(),
            output: "{}".to_string(),
            duration_ms: 0,
        }
    }
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingSink {
    fn events_named(&self, name: &str) -> Vec<TurnStreamEvent> {
        self.events
            .lock()
            .expect("F2：recording sink 锁被毒化")
            .iter()
            .filter(|(captured_name, _)| captured_name == name)
            .map(|(_, payload)| payload.clone())
            .collect()
    }

    fn names(&self) -> Vec<String> {
        self.events
            .lock()
            .expect("F2：recording sink 锁被毒化")
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }
}

impl TurnEventSink for RecordingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events
            .lock()
            .expect("F2：recording sink 锁被毒化")
            .push((name.to_string(), payload));
    }
}

/// buffered（非流式）provider：`supports_streaming=false` ⇒ 决策走同步链，
/// `first_token_latency_ms` 必然缺席 ⇒ 生成窗口 = 调用墙钟（失败耗时无处躲藏）。
fn buffered_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "f2-retry-window".to_string(),
        provider_name: "f2-retry-window".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "F2_RETRY_WINDOW_API_KEY".to_string(),
        api_key: Some("f2-dummy-key".to_string()),
        model: "f2-retry-window-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 256,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: false,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn turn_input(session_id: &str) -> TurnInput {
    TurnInput {
        message: "f2 retry window probe".to_string(),
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

/// 跑一轮完整 turn，返回 `(终态事件 payload, mock 侧实际收到的请求数)`。
fn run_single_turn_with_retry(provider: &ScriptedProvider, session_id: &str) -> (TurnStreamEvent, usize) {
    let temp_file = std::env::temp_dir().join(format!(
        "pony-f2-retry-window-{}-{}.db",
        std::process::id(),
        session_id
    ));
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::with_backend(Box::new(FileSessionBackend::new(temp_file.clone()))),
        Box::new(StaticResolver {
            selection: buffered_selection(provider.base_url.clone()),
        }),
        Box::new(NoopExecutor),
        Box::new(LocalTurnPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    let sink = RecordingSink::default();
    runtime.start_turn_stream(
        &sink,
        format!("turn-{session_id}"),
        turn_input(session_id),
    );
    let _ = std::fs::remove_file(&temp_file);

    let completed = sink
        .events_named("turn:completed")
        .pop()
        .unwrap_or_else(|| {
            panic!(
                "F2：一轮必须产出 turn:completed；实际事件序列 = {:?}",
                sink.names()
            )
        });
    (completed, provider.request_count())
}

// ---------------------------------------------------------------------------
// JSON 展平（与 F1 同构；键名语义匹配、不绑嵌套深度）
// ---------------------------------------------------------------------------

fn flatten(value: &Value, prefix: &str, out: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                flatten(child, &path, out);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                flatten(child, &format!("{prefix}[{index}]"), out);
            }
        }
        leaf => out.push((prefix.to_string(), leaf.clone())),
    }
}

fn flattened(payload: &TurnStreamEvent) -> Vec<(String, Value)> {
    let json = serde_json::to_value(payload).expect("F2：终态事件 payload 必须可序列化");
    let mut out = Vec::new();
    flatten(&json, "", &mut out);
    out
}

/// 末段键名（去掉数组下标）用于语义匹配。
fn last_key(path: &str) -> &str {
    let without_index = match path.rfind('[') {
        Some(index) => &path[..index],
        None => path,
    };
    without_index.rsplit('.').next().unwrap_or(without_index)
}

fn single_call_record(completed: &TurnStreamEvent) -> &ProviderCallCacheRecord {
    let records = completed
        .provider_call_records
        .as_ref()
        .unwrap_or_else(|| panic!("F2：终态事件必须携带 providerCallRecords（per-call 取证面）"));
    assert_eq!(
        records.len(),
        1,
        "F2 夹具前提：本剧本只有一次成功调用（initial 决策），实际 {} 条记录",
        records.len()
    );
    &records[0]
}

// ---------------------------------------------------------------------------
// I2/I3：生成窗口必须只覆盖最后一次成功 attempt
// ---------------------------------------------------------------------------

#[test]
fn f2_generation_window_excludes_failed_attempt() {
    let _guard = f2_lock();
    // Arrange：第一次 attempt 500ms 级失败（X=700ms），第二次成功。
    let provider = ScriptedProvider::script(vec![
        MockReply {
            status: 503,
            reason: "Service Unavailable",
            body: SERVER_ERROR_BODY,
            content_type: "application/json",
            delay: Duration::from_millis(FAILED_ATTEMPT_MS),
        },
        MockReply {
            status: 200,
            reason: "OK",
            body: OPENAI_OK_BODY,
            content_type: "application/json",
            delay: Duration::ZERO,
        },
    ]);

    // Act
    let started = Instant::now();
    let (completed, requests) = run_single_turn_with_retry(&provider, "session-f2-window");
    let wall_clock = started.elapsed();

    // Assert ①：确实走过了「失败 → 重试 → 成功」的真实链路（否则下面的断言在证明空气）。
    assert!(
        requests >= 2,
        "F2 反向钉子：503 属既有可重试瞬时错误，必须真实重试（≥2 次 HTTP），实际 {requests} 次；\
         不得用「禁掉重试」的方式规避窗口污染（wall_clock={wall_clock:?}）"
    );
    assert!(
        wall_clock >= Duration::from_millis(FAILED_ATTEMPT_MS),
        "F2 夹具前提：整轮墙钟必须 ≥ 失败 attempt 的 X={FAILED_ATTEMPT_MS}ms，实际 {wall_clock:?}"
    );

    // Assert ①′：mock 侧确实收到了 **2 个完整请求体**（失败 attempt 与成功 attempt 各一）。
    let bodies = provider.received_bodies();
    assert_eq!(
        bodies.len(),
        requests,
        "F2 证据面：mock 解析到的请求体条数（{})必须与实际请求数（{requests}）一致",
        bodies.len()
    );
    assert!(
        bodies
            .iter()
            .all(|body| body.get("model").is_some() && body.get("messages").is_some()),
        "F2 证据面：两次 attempt 都必须是完整 chat/completions 请求体，实际 = {bodies:?}"
    );

    let record = single_call_record(&completed);

    // Assert ②：记录下来的生成窗口不得包含 X（只计最后一次成功 attempt）。
    let duration = record.turn_duration_ms.unwrap_or_else(|| {
        panic!("F2：per-call 记录必须带 turn_duration_ms（窗口取证的唯一载体），实际 None")
    });
    let ttft = record.first_token_latency_ms;
    let generation_window = duration.saturating_sub(ttft.unwrap_or(0));
    assert!(
        generation_window < GENERATION_WINDOW_BOUND_MS,
        "F2 I2/I3：生成窗口必须只覆盖最后一次成功 attempt（上界 {GENERATION_WINDOW_BOUND_MS}ms），\
         实际 gen={generation_window}ms（dur={duration}ms, ttft={ttft:?}）——\
         失败 attempt 的 X={FAILED_ATTEMPT_MS}ms（外加退避睡眠）被计进了分母"
    );

    // Assert ③：分母干净 ⇒ 同一批 token 的速度不得被失败尝试系统性压低。
    //   40 output tokens / 干净窗口 ⇒ 远超 100 t/s；被 700ms+X 污染后只剩 ~30 t/s 量级。
    let speed = completed
        .metrics
        .as_ref()
        .and_then(|metrics| metrics.turn_speed_tokens_per_second)
        .unwrap_or_else(|| {
            panic!(
                "F2：终态事件必须携带 metrics.turnSpeedTokensPerSecond（口径取证面），实际缺失；\
                 metrics={:?}",
                completed.metrics
            )
        });
    assert!(
        speed > 100.0,
        "F2 I3：窗口纯净时 {SUCCESS_OUTPUT_TOKENS} tokens 的轮速度必须 >100 t/s，实际 {speed} t/s\
         （窗口含失败 attempt 的指纹：~{deflated:.1} t/s）",
        deflated = (SUCCESS_OUTPUT_TOKENS as f64) / ((FAILED_ATTEMPT_MS + 500) as f64 / 1000.0)
    );
}

// ---------------------------------------------------------------------------
// I2：attempts 次数与失败耗时必须作为独立字段可读出（wire 载荷 **或** 结构化日志任一载体）
// ---------------------------------------------------------------------------

#[test]
fn f2_attempt_count_is_readable_as_independent_field() {
    let _guard = f2_lock();
    // Arrange：同一剧本（一次失败 + 一次成功）；注入日志落盘路径作为取证载体。
    let log_path = temp_log_path("attempts");
    let _ = std::fs::remove_file(&log_path);
    set_log_file_path(log_path.clone());
    let provider = ScriptedProvider::script(vec![
        MockReply {
            status: 503,
            reason: "Service Unavailable",
            body: SERVER_ERROR_BODY,
            content_type: "application/json",
            delay: Duration::from_millis(FAILED_ATTEMPT_MS),
        },
        MockReply {
            status: 200,
            reason: "OK",
            body: OPENAI_OK_BODY,
            content_type: "application/json",
            delay: Duration::ZERO,
        },
    ]);

    // Act
    let (completed, requests) = run_single_turn_with_retry(&provider, "session-f2-attempts");
    let lines = log_lines(&log_path);
    let _ = std::fs::remove_file(&log_path);

    // Assert ①：mock 侧真值 = 2 次 HTTP（1 次 503 + 1 次 200）。
    assert_eq!(
        requests, 2,
        "F2 夹具前提：剧本应为「1 次 503 + 1 次 200」= 2 次 HTTP，实际 {requests} 次"
    );

    // Assert ②：终态载荷或结构化日志里，必须能按语义读出「attempts = 2」。
    let wire_flat = flattened(&completed);
    let log_flats: Vec<Vec<(String, Value)>> = lines
        .iter()
        .map(|line| {
            let mut flat = Vec::new();
            flatten(line, "", &mut flat);
            flat
        })
        .collect();
    let wire_attempts = attempts_evidence(&wire_flat, 2);
    let log_attempts = log_flats
        .iter()
        .find_map(|flat| attempts_evidence(flat, 2).map(|path| (path, flat.clone())));
    assert!(
        wire_attempts.is_some() || log_attempts.is_some(),
        "契约 I2：attempts 次数必须作为**独立字段**记录并可读出（wire 载荷或结构化日志），\
         实际 wire 命中 = {wire_attempts:?}，日志行数 = {}，日志里含 attempt 的键 = {:?}",
        log_flats.len(),
        log_flats
            .iter()
            .flat_map(|flat| {
                flat.iter()
                    .filter(|(path, _)| last_key(path).to_lowercase().contains("attempt"))
                    .map(|(path, value)| format!("{path}={value}"))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    );

    // Assert ③：同一载体里必须同时能读出「失败 attempt 自身耗时」> 0（独立字段，不得混入 gen）。
    let failed_ms = match log_attempts.as_ref() {
        Some((_, flat)) => failed_attempt_evidence(flat),
        None => failed_attempt_evidence(&wire_flat),
    }
    .or_else(|| log_flats.iter().find_map(|flat| failed_attempt_evidence(flat)));
    let (failed_path, failed_ms) = failed_ms.unwrap_or_else(|| {
        panic!(
            "契约 I2：失败 attempt 的自身耗时必须作为独立字段记录（且 > 0），实际未找到；\
             日志行数 = {}",
            lines.len()
        )
    });
    assert!(
        failed_ms >= FAILED_ATTEMPT_MS * 8 / 10,
        "契约 I2：独立字段 `{failed_path}` 必须反映真实失败耗时（≥ {lower}ms，X={FAILED_ATTEMPT_MS}ms），实际 {failed_ms}ms",
        lower = FAILED_ATTEMPT_MS * 8 / 10
    );
}
