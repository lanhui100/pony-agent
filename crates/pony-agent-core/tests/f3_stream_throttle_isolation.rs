//! [diag-tps / task-6] F3「节流离开 SSE 读线程」验收（独立测试者 L2-T，红相先写、冻结）。
//!
//! 契约真值源（Lead 冻结，只读）：共享任务板 `task-6` 的 I1–I4 + 诊断综合
//! `.dev-team/diag-tps/00-SYNTHESIS.md` §3 缺陷 1（`MIN_CLIENT_GAP_MS=16` 的同步 sleep 在
//! 阻塞 SSE 读循环的 `on_delta` 内联执行 ⇒ 睡眠 = 停止读 socket = TCP 背压）。
//!
//! 判据口径（**契约 v2，Lead 2026-11-12 裁决**；只断言时序性质，不绑实现形态）：
//! - **T1（必绿）静态断言**：SSE 读路径源码里不得再出现 `thread::sleep` / `MIN_CLIENT_GAP`
//!   ——「读循环内零阻塞节流」的机械可查形态（F3-I1′/I3′-①）。
//! - **T2（必绿）行为断言**：快 sink + N=80 个 0ms 间隔的 1-token chunk；整轮耗时不得被
//!   16ms/emit 的节流钉在 ~62.5 emits/s 量级（F3-I3′-②）。
//! - **T3（必绿）语义不丢**：合并允许，但**最终帧与终态不得丢失**——最后一帧文本必须出现在
//!   delta 流（或终态正文）中，且一轮恰好一条 `turn:completed`。
//! - **T1b（`#[ignore]`，ADVISORY）慢 sink 解耦**：注入「每次消费 sleep 50ms」的 sink 时，
//!   整轮耗时仍会随 sink 线性增长 ⇒ **结构性限制登记**（v2 裁决：`sink.emit` 内联属受
//!   sink 契约保护的允许行为——Tauri emit 是 fire-and-forget、TUI 是无界 channel、headless
//!   无消费者；把 emit 挪到独立线程属几十行并发改造 + 丢帧/终态风险，本轮不做）。
//!   该用例**不参与门禁**，只作将来解耦改造的红相锚点（登记于红相收据「已知限制」）。
//!
//! 全部经**本地回环 mock SSE provider**（`127.0.0.1:0`，零外网）驱动；sink 的 sleep 是被测输入
//! （模拟慢客户端消费），不是「等待同步」。时序断言取**绝对上界**并在失败信息里打印实测值，
//! 便于基线复核与 CI 抖动定位。
//!
//! 纪律（test-expert L2）：AAA、固定剧本、无随机、无 `#[ignore]`、每条用例均有非空断言。
//! 红相锚定：当前基线把 sink 的 50ms 睡眠串进读循环（T1）/ 每 emit 内联 sleep 16ms（T2）
//! ⇒ 两个时序用例必失败（断言失败，非编译错误）。
//!
//! 本文件由 Test Agent 独占写域。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::context::DefaultTurnContextBuilder;
use pony_agent_core::agent::planner::LocalTurnPlanner;
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntime, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::{FileSessionBackend, SessionStore};
use pony_agent_core::agent::telemetry::DefaultTurnTelemetryBuilder;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutor, ToolResult};
use pony_agent_core::agent::turn_flow::TurnEventSink;

/// 慢客户端 sink 的单次消费耗时。
const SLOW_SINK_MS: u64 = 50;
/// 慢客户端用例的 delta 数（线性基线 = N × 50ms = 2000ms）。
const SLOW_SINK_DELTAS: usize = 40;
/// T1 上界 = 线性基线的 50%（=1000ms）：允许固定开销与少量合并帧，禁止随 N 线性叠加。
const SLOW_SINK_BOUND_MS: u64 = SLOW_SINK_DELTAS as u64 * SLOW_SINK_MS / 2;
/// 快客户端用例的 delta 数（16ms/emit 节流基线 ≈ N × 16ms = 1280ms）。
const FAST_SINK_DELTAS: usize = 80;
/// T2 上界：远低于 16ms/emit 的节流指纹（1280ms），又给固定开销留出余量。
const FAST_SINK_BOUND_MS: u64 = 600;

/// 最后一帧的文本标记（合并允许，丢失不允许）。
fn token_marker(index: usize) -> String {
    format!("TOK{index:03}")
}

// ---------------------------------------------------------------------------
// 回环 mock SSE provider
// ---------------------------------------------------------------------------

/// 一次性剧本 server：接受连接 → 读完请求 → 回 `text/event-stream`，逐帧 `data: {...}`，
/// 帧间**不插入任何延迟**（模拟「1 token/chunk、间隔中位 0ms」的突变路由）。
struct MockSseProvider {
    base_url: String,
    addr: String,
    shutdown: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl MockSseProvider {
    fn start(delta_count: usize) -> Self {
        assert!(delta_count > 0, "F3：delta 数必须为正");
        let listener = TcpListener::bind("127.0.0.1:0").expect("F3：mock provider 必须能绑定回环端口");
        let port = listener
            .local_addr()
            .expect("F3：mock provider 必须能取本地地址")
            .port();
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_for_thread = Arc::clone(&shutdown);
        let mut body = String::new();
        for index in 0..delta_count {
            body.push_str(&format!(
                "data: {{\"choices\":[{{\"index\":0,\"delta\":{{\"content\":\"{} \"}}}}]}}\n\n",
                token_marker(index)
            ));
        }
        // usage 帧 + 终止符（既有 SSE 解析契约）。
        body.push_str(&format!(
            "data: {{\"choices\":[],\"usage\":{{\"prompt_tokens\":10,\"completion_tokens\":{delta_count},\"total_tokens\":{}}}}}\n\n",
            10 + delta_count
        ));
        body.push_str("data: [DONE]\n\n");

        let handle = std::thread::spawn(move || {
            while !shutdown_for_thread.load(Ordering::SeqCst) {
                let Ok((mut stream, _addr)) = listener.accept() else {
                    return;
                };
                read_http_request(&mut stream);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.as_bytes().len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body.as_bytes());
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                drain_incoming(&mut stream);
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1"),
            addr: format!("127.0.0.1:{port}"),
            shutdown,
            handle: Some(handle),
        }
    }
}

impl Drop for MockSseProvider {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(self.addr.clone());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn read_http_request(stream: &mut TcpStream) {
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
// Sink：可注入「客户端消费速度」
// ---------------------------------------------------------------------------

struct ConsumerSink {
    /// 每次消费 `turn:delta` 的耗时（0ms = 快客户端）。
    per_delta_delay: Duration,
    events: Mutex<Vec<(String, TurnStreamEvent)>>,
    /// 已消费的 delta 文本（合并帧可能一次带来多段，按到达顺序拼接）。
    delta_texts: Mutex<Vec<String>>,
    delta_emits: Mutex<usize>,
}

impl ConsumerSink {
    fn new(per_delta_delay: Duration) -> Self {
        Self {
            per_delta_delay,
            events: Mutex::new(Vec::new()),
            delta_texts: Mutex::new(Vec::new()),
            delta_emits: Mutex::new(0),
        }
    }

    fn events_named(&self, name: &str) -> Vec<TurnStreamEvent> {
        self.events
            .lock()
            .expect("F3：sink events 锁被毒化")
            .iter()
            .filter(|(captured_name, _)| captured_name == name)
            .map(|(_, payload)| payload.clone())
            .collect()
    }

    fn names(&self) -> Vec<String> {
        self.events
            .lock()
            .expect("F3：sink events 锁被毒化")
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn joined_delta_text(&self) -> String {
        self.delta_texts
            .lock()
            .expect("F3：sink delta 锁被毒化")
            .join("")
    }

    fn delta_emit_count(&self) -> usize {
        *self.delta_emits.lock().expect("F3：sink 计数锁被毒化")
    }
}

impl TurnEventSink for ConsumerSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        let is_delta = name == "turn:delta";
        if is_delta {
            // 先记账（在锁外 sleep，避免把「慢消费」变成「互斥锁占用」）。
            if let Some(text) = payload.text.clone() {
                self.delta_texts
                    .lock()
                    .expect("F3：sink delta 锁被毒化")
                    .push(text);
            }
            *self.delta_emits.lock().expect("F3：sink 计数锁被毒化") += 1;
        }
        self.events
            .lock()
            .expect("F3：sink events 锁被毒化")
            .push((name.to_string(), payload));
        if is_delta && !self.per_delta_delay.is_zero() {
            // 慢客户端：消费一帧要 50ms（读循环若内联等待它，就会停止读 socket）。
            std::thread::sleep(self.per_delta_delay);
        }
    }
}

// ---------------------------------------------------------------------------
// runtime 装配
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

fn streaming_selection(base_url: String) -> ResolvedProviderSelection {
    ResolvedProviderSelection {
        requested_name: "f3-stream-throttle".to_string(),
        provider_name: "f3-stream-throttle".to_string(),
        protocol: ProviderProtocol::OpenAiCompletions,
        base_url,
        auth_type: ProviderAuthType::Auto,
        api_key_env_var: "F3_STREAM_THROTTLE_API_KEY".to_string(),
        api_key: Some("f3-dummy-key".to_string()),
        model: "f3-stream-throttle-model".to_string(),
        temperature: 0.0,
        max_output_tokens: 256,
        reasoning_effort: None,
        reasoning_budget_tokens: None,
        capabilities: ProviderModelCapabilities {
            context_window_tokens: Some(128_000),
            supports_tools: true,
            supports_streaming: true,
            supports_image_input: false,
            supports_reasoning: false,
            ..Default::default()
        },
        thinking_param_pattern: ThinkingParamPattern::EffortStandard,
    }
}

fn turn_input(session_id: &str) -> TurnInput {
    TurnInput {
        message: "f3 stream throttle probe".to_string(),
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

/// 跑一轮；返回整轮墙钟耗时。
fn run_streamed_turn(provider: &MockSseProvider, sink: &ConsumerSink, tag: &str) -> Duration {
    let temp_file = std::env::temp_dir().join(format!(
        "pony-f3-throttle-{}-{}.db",
        std::process::id(),
        tag
    ));
    let mut runtime = AgentRuntime::with_dependencies(
        SessionStore::with_backend(Box::new(FileSessionBackend::new(temp_file.clone()))),
        Box::new(StaticResolver {
            selection: streaming_selection(provider.base_url.clone()),
        }),
        Box::new(NoopExecutor),
        Box::new(LocalTurnPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    let started = Instant::now();
    runtime.start_turn_stream(sink, format!("turn-{tag}"), turn_input(&format!("session-{tag}")));
    let elapsed = started.elapsed();
    let _ = std::fs::remove_file(&temp_file);
    elapsed
}

/// 有界条件轮询（**不是**盲等 sleep）：直到谓词成立或到达硬截止。
fn wait_until(deadline: Duration, predicate: impl Fn() -> bool) -> bool {
    let started = Instant::now();
    loop {
        if predicate() {
            return true;
        }
        if started.elapsed() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// ---------------------------------------------------------------------------
// T1（必绿）：静态断言 —— SSE 读路径内不得再有阻塞式节流
// ---------------------------------------------------------------------------

/// 参与「SSE 读路径」的三份源码（相对 `crates/pony-agent-core`）。
const SSE_READ_PATH_SOURCES: [&str; 3] = [
    "src/agent/runtime/turn_stream.rs",
    "src/agent/runtime/tool_recovery.rs",
    "src/agent/provider/openai_sse.rs",
];

/// 去掉行内 `//` 注释后的代码行（注释里写「旧实现曾 sleep」不算违规；
/// 本函数只为静态扫描服务，不追求 Rust 词法完备）。
fn strip_line_comment(line: &str) -> &str {
    match line.find("//") {
        Some(index) => &line[..index],
        None => line,
    }
}

#[test]
fn f3_sse_read_path_has_no_blocking_throttle() {
    // Arrange：被测源码根目录（integration test 的 CARGO_MANIFEST_DIR = crates/pony-agent-core）。
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut scanned: Vec<(String, Vec<(usize, String)>)> = Vec::new();

    // Act：逐份读取并筛出**代码行**（注释剥离）里的阻塞原语。
    for relative in SSE_READ_PATH_SOURCES {
        let path = root.join(relative);
        let source = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("F3 静态断言：必须能读取 SSE 读路径源码 `{}`：{error}", path.display())
        });
        assert!(
            source.lines().count() > 100,
            "F3 静态断言：`{}` 行数 {} 过少，疑似读错文件（不得空转通过）",
            path.display(),
            source.lines().count()
        );
        let mut violations: Vec<(usize, String)> = Vec::new();
        for (index, line) in source.lines().enumerate() {
            let code = strip_line_comment(line);
            if code.contains("thread::sleep")
                || code.contains("MIN_CLIENT_GAP")
                || code.contains(".sleep(")
            {
                violations.push((index + 1, code.trim().to_string()));
            }
        }
        scanned.push((relative.to_string(), violations));
    }

    // Assert：三份源码的 SSE 读路径内零阻塞节流（契约 F3-I1′ / I3′-①）。
    let offenders: Vec<String> = scanned
        .iter()
        .filter(|(_, violations)| !violations.is_empty())
        .map(|(file, violations)| {
            format!(
                "{file}: {}",
                violations
                    .iter()
                    .map(|(line, text)| format!("L{line} `{text}`"))
                    .collect::<Vec<_>>()
                    .join(" | ")
            )
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "契约 F3-I1′/I3′-①：SSE 读路径（on_delta 闭包 + openai_sse::push_payload 调用链）\
         内不得再出现 `thread::sleep` / `MIN_CLIENT_GAP` / 异步 `.sleep(`，实际命中：\n{}",
        offenders.join("\n")
    );
}

// ---------------------------------------------------------------------------
// T1：读循环不得被客户端消费速度阻塞
// ---------------------------------------------------------------------------

/// ADVISORY / 已知限制（契约 v2 降级，**不参与门禁**，故 `#[ignore]`）：
///
/// 结构性限制：`sink.emit` 在读线程内联执行（生产 sink 均为非阻塞投递——Tauri emit
/// fire-and-forget、TUI 无界 channel、headless 无消费者），所以「慢客户端」不是生产故障面；
/// 但若将来某个宿主 sink 变成阻塞式，读循环会被其消费速度拖住（本用例即该形态的锚点）。
/// 彻底解耦需把 emit 移到独立发送线程（几十行并发改造 + 丢帧/终态风险）⇒ 走 backlog，
/// 本轮不做（Lead 2026-11-12 裁决）。
///
/// 运行方式（仅诊断）：`cargo test -p pony-agent-core --test f3_stream_throttle_isolation -- --ignored`
#[test]
#[ignore = "ADVISORY/已知限制（契约 v2）：sink.emit 内联属受 sink 契约保护的允许行为；彻底解耦走 backlog"]
fn f3_read_loop_is_not_gated_by_slow_client_consumption() {
    // Arrange：40 个 delta 的突发 SSE；客户端每消费一帧 sleep 50ms（线性基线 2000ms）。
    let provider = MockSseProvider::start(SLOW_SINK_DELTAS);
    let sink = ConsumerSink::new(Duration::from_millis(SLOW_SINK_MS));

    // Act
    let elapsed = run_streamed_turn(&provider, &sink, "slow-consumer");

    // Assert ①：整轮耗时不随 N 线性叠加 sink 的睡眠。
    assert!(
        elapsed < Duration::from_millis(SLOW_SINK_BOUND_MS),
        "F3 I1/I3：读循环处理单个 delta 不得被客户端消费速度阻塞——\
         整轮耗时必须 < {SLOW_SINK_BOUND_MS}ms（线性基线 = {SLOW_SINK_DELTAS} × {SLOW_SINK_MS}ms = {}ms 的 50%），\
         实际 {elapsed:?}；慢客户端实际消费了 {} 帧 delta",
        SLOW_SINK_DELTAS as u64 * SLOW_SINK_MS,
        sink.delta_emit_count()
    );

    // Assert ②：合并允许，最终帧不得丢（读循环没把尾部帧吞掉/丢弃）。
    let last_marker = token_marker(SLOW_SINK_DELTAS - 1);
    let delivered = wait_until(Duration::from_secs(3), || {
        sink.joined_delta_text().contains(&last_marker)
    });
    assert!(
        delivered,
        "F3 I2：允许合并中间帧，但最终帧不得丢失——最后一帧 `{last_marker}` 必须出现在客户端消费流中，\
         实际已消费 delta 帧数 = {}，拼接文本前 200 字符 = {:?}",
        sink.delta_emit_count(),
        sink.joined_delta_text().chars().take(200).collect::<String>()
    );

    // Assert ③：终态不得丢，且一轮恰好一条。
    let completed = sink.events_named("turn:completed");
    assert_eq!(
        completed.len(),
        1,
        "F3 I2：一轮必须恰好一条 `turn:completed`（终态不得丢/不得重复），实际 {} 条；事件序列 = {:?}",
        completed.len(),
        sink.names()
    );
    let completed_text = completed[0].text.clone().unwrap_or_default();
    assert!(
        completed_text.contains(&last_marker),
        "F3 I2：终态正文必须包含完整累计文本（含最后一帧 `{last_marker}`），实际 = {:?}",
        completed_text.chars().take(200).collect::<String>()
    );
}

// ---------------------------------------------------------------------------
// T2：16ms/emit 的内联节流不得把 0ms 间隔的突变路由掐在 62.5 emits/s
// ---------------------------------------------------------------------------

#[test]
fn f3_zero_gap_burst_is_not_capped_by_inline_throttle() {
    // Arrange：80 个 1-token chunk、帧间隔 0ms（诊断裁决里的「细粒度路由」指纹）；快客户端。
    let provider = MockSseProvider::start(FAST_SINK_DELTAS);
    let sink = ConsumerSink::new(Duration::ZERO);

    // Act
    let elapsed = run_streamed_turn(&provider, &sink, "fast-consumer");

    // Assert ①：不得被 16ms/emit 节流钉住（基线指纹 ≈ 80 × 16ms ≈ 1280ms）。
    assert!(
        elapsed < Duration::from_millis(FAST_SINK_BOUND_MS),
        "F3 I1/I3：0ms 间隔的突变路由不得被读循环内的 16ms/emit 节流掐在 ~62.5 emits/s——\
         整轮耗时必须 < {FAST_SINK_BOUND_MS}ms（节流指纹 ≈ {}ms），实际 {elapsed:?}；\
         客户端侧 delta 帧数 = {}",
        FAST_SINK_DELTAS as u64 * 16,
        sink.delta_emit_count()
    );

    // Assert ②：最终帧与终态不丢。
    let last_marker = token_marker(FAST_SINK_DELTAS - 1);
    let delivered = wait_until(Duration::from_secs(3), || {
        sink.joined_delta_text().contains(&last_marker)
    });
    assert!(
        delivered,
        "F3 I2：节流改造后最终帧不得丢失——`{last_marker}` 未到达客户端；已消费 {} 帧",
        sink.delta_emit_count()
    );
    let completed = sink.events_named("turn:completed");
    assert_eq!(
        completed.len(),
        1,
        "F3 I2：一轮必须恰好一条 `turn:completed`，实际 {} 条；事件序列 = {:?}",
        completed.len(),
        sink.names()
    );
    assert!(
        completed[0]
            .text
            .as_deref()
            .unwrap_or_default()
            .contains(&last_marker),
        "F3 I2：终态正文必须完整（含最后一帧 `{last_marker}`），实际 = {:?}",
        completed[0].text
    );
}
