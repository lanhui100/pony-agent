//! Wave-45 · OTel resource 归因（PA-153）+ `agent.turn` 根 span 错误承接（PA-154）验收测试（红相）。
//!
//! 本文件是 **Test Agent（L2-AT）独占写域**，只读契约、不得由实现方修改。契约来源（唯一口径）：
//! `.dev-team/contract-matrix-wave45.md` §0（事实锚点 F1–F16）、§1（PA-153）、§2（PA-154）、§6（冻结 id）。
//! 缺陷事实来源：`docs/audit/observability-audit-2026-10-25.md` O1/O2（P1-1 根 span 不承接错误、
//! P1-4 resource 无版本且环境恒为 production）。
//!
//! 覆盖（断言 id 严格对齐契约 §6 冻结清单）：
//! - **PA153-T1**（§1 T1）：一条导出 ResourceSpans 同时含 `service.name` / `service.version` /
//!   `deployment.environment`（各一次）；`service.version` 默认等于 core crate 版本
//!   （测试 crate 与库同包同版本 ⇒ `env!("CARGO_PKG_VERSION")`）；debug 构建下
//!   `deployment.environment` 值为 `development`。含非空洞锚：body 必须同时含 `service.name` 与版本字面量。
//! - **PA153-T2**（§1 T2，env 覆盖面）：落在**独立测试二进制**
//!   `tests/otel_resource_env_test.rs`（Lead 裁决：进程隔离 ⇒ 每进程一个确定环境态，
//!   避免 `set_var` 串味污染本文件的 resource 断言）。本文件不写进程 env，id 不重复声明。
//! - **PA154-T1**（§2 T1 + Gate-1 A 契约演进）：failed tool span + `phase="completed"` ⇒
//!   根 span `status.message` = `turn_error: tool_failed`（唯一前缀 + 归因标签），该前缀子串
//!   **恰好 1 次**（子 span 既有文案不得含 `turn_error`）；`agent.turn.outcome` 值必须为 `error`。
//! - **PA154-T2**（§2 T2 + Gate-1 A）：治理 block（`record_verdict(..., "block", ...)`）+
//!   `phase="completed"` ⇒ 根 span `status.message` = `turn_error: quota_blocked`
//!   （不得只靠裸 `quota_blocked`，那与 HEAD 既有 governance 子 span 混淆）；outcome 值 = `error`。
//! - **PA154-T3**（§2 T3）：全成功（tool `success`、无 block、`phase="completed"`）⇒
//!   `agent.turn.outcome` / `tool_failed` / `quota_blocked` 均**不出现**，且必含
//!   `agent.turn` + `agent.turn.phase` + `completed`（非空洞）。
//! - **PA154-ADV**（对抗 + Gate-1 A #4）：无 child error 且 `phase="failed"` ⇒ 既有语义不变
//!   （body 含 `failed`）、**不含** `turn_error`（phase 优先）、仍含 `agent.turn.outcome`（值 `error`）、
//!   不得 panic、不得虚构 `tool_failed` 归因。
//!
//! 断言纪律（契约 §0/§6）：
//! - 只断言 **wire 字面量**，不引用任何待新增符号（否则红相会退化成编译失败）；
//! - 键名一律写字面量，另设一条常量冻结测试防「常量被改名后自指自证」；
//! - 负向断言必配正向锚（body 确实承载了本轮 span/phase），杜绝「导出为空 ⇒ 假绿」；
//! - 禁止 `#[ignore]`、禁止无断言、禁止 `unwrap()` 吞错、禁止无界等待（sink 有硬截止）。
//!
//! 确定性：本地 `127.0.0.1` 一次性 OTLP/HTTP sink（零外网）；全局门禁串行锁 + 每条用例
//! 自建/自毁 provider。本文件与 `otel_trace_migration_test.rs` 属不同测试二进制（各自独立进程），
//! 全局 `AtomicBool` 门禁互不污染。
//!
//! 作者：test-agent-45（L2-AT）。红相锚定后本文件冻结只读，任何修改须 Lead 批准。

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::otel::{self, ToolSpanInput, DEFAULT_SAMPLING_RATIO};

// ---------------------------------------------------------------- 门禁串行锁
//
// 全局 provider / 上报门禁 / 进程 env 都是进程级状态；cargo test 默认多线程并发跑用例。
// 凡触达 otel 全局态的用例必须持锁串行，且结尾把门禁复位为 false。

static GATE_LOCK: Mutex<()> = Mutex::new(());

fn gate_lock() -> MutexGuard<'static, ()> {
    GATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// -------------------------------------------------- 本地 OTLP/HTTP sink（探针）
//
// 非阻塞 accept + 固定轮询 + 硬截止（仓内既有验收惯例，禁止无界等待）；捕获
// method/path/body 后回 `200 + 空 ExportTraceServiceResponse`。

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    body: Vec<u8>,
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack.windows(needle.len()).position(|window| window == needle)
}

fn read_http_request(stream: &mut TcpStream) -> Vec<u8> {
    let mut raw: Vec<u8> = Vec::new();
    let mut buffer = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_millis(1_000);
    let mut expected_total: Option<usize> = None;

    while Instant::now() < deadline {
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
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(_) => break,
        }
    }

    raw
}

fn parse_captured_request(raw: &[u8]) -> CapturedRequest {
    let header_end = find_subslice(raw, b"\r\n\r\n").unwrap_or(raw.len());
    let head = String::from_utf8_lossy(&raw[..header_end]);
    let mut parts = head.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let body = if header_end + 4 <= raw.len() {
        raw[header_end + 4..].to_vec()
    } else {
        Vec::new()
    };
    CapturedRequest { method, path, body }
}

fn spawn_otlp_sink(budget: Duration) -> (u16, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("本地 OTLP sink 必须能绑定回环端口");
    listener
        .set_nonblocking(true)
        .expect("本地 OTLP sink 必须为非阻塞监听");
    let port = listener.local_addr().expect("sink local_addr").port();

    let handle = std::thread::spawn(move || {
        let mut captured: Vec<CapturedRequest> = Vec::new();
        let deadline = Instant::now() + budget;
        while Instant::now() < deadline {
            match listener.accept() {
                Ok((mut stream, _peer)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(250)));
                    let raw = read_http_request(&mut stream);
                    let _ = stream.write_all(
                        b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\nconnection: close\r\n\r\n",
                    );
                    let _ = stream.flush();
                    captured.push(parse_captured_request(&raw));
                }
                Err(err) if err.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
        captured
    });

    (port, handle)
}

fn local_config(port: u16) -> otel::OtelConfig {
    otel::OtelConfig {
        endpoint: format!("http://127.0.0.1:{port}"),
        service_name: "pony-agent-wave45-acceptance".to_string(),
        sampling_ratio: DEFAULT_SAMPLING_RATIO,
        headers: Vec::new(),
    }
}

/// 把捕获到的全部 `POST /v1/traces` body 拼成一段 wire 文本。
///
/// protobuf 的 string/bytes 字段按 UTF-8 原样落在字节里，ASCII 键/值可直接子串匹配
///（契约 §2「可观测代理说明」冻结的口径：不做 protobuf 结构解析）。
fn wire_text(captured: &[CapturedRequest]) -> String {
    assert!(
        !captured.is_empty(),
        "本次窗口内必须至少收到一个 OTLP 导出请求（否则后续 wire 断言全部是假绿）"
    );
    let bodies: Vec<String> = captured
        .iter()
        .map(|request| {
            assert_eq!(request.method, "POST", "OTLP/HTTP 导出必须为 POST");
            assert!(
                request.path.contains("/v1/traces"),
                "OTLP/HTTP trace 导出路径必须为 /v1/traces，实际 `{}`",
                request.path
            );
            assert!(
                !request.body.starts_with(&[0x1f, 0x8b]),
                "导出 body 不得为 gzip（若改 feature 须先补解压断言，否则子串断言失效）"
            );
            String::from_utf8_lossy(&request.body).to_string()
        })
        .collect();
    bodies.join("\n")
}

/// Arrange 段：起 sink（零外网）→ init 本地回环 → 开门禁。
fn start_local_pipeline(budget_ms: u64) -> JoinHandle<Vec<CapturedRequest>> {
    otel::shutdown();
    otel::set_reporting_enabled(false);
    let (port, sink) = spawn_otlp_sink(Duration::from_millis(budget_ms));
    otel::init(local_config(port)).expect("D3/A3：本地回环端点必须可装配（装配失败只降级不 panic）");
    otel::set_reporting_enabled(true);
    sink
}

/// Assert 段：flush 必须有界成功 → shutdown → 收 sink → 返回 wire 文本。
fn drain_wire(sink: JoinHandle<Vec<CapturedRequest>>) -> String {
    let flushed = otel::force_flush(3_000);
    assert!(flushed.is_ok(), "已 init 时 force_flush 必须返回 Ok：{flushed:?}");
    otel::shutdown();
    otel::set_reporting_enabled(false);
    let captured = sink.join().expect("sink 线程不得 panic");
    wire_text(&captured)
}

// ---------------------------------------------------- wire 级断言小工具（PA-154）

/// `turn_error: ` 统一前缀（Gate-1 A 契约演进）：根 span `status.message` 的唯一前缀，
/// 与子 span 既有文案（`tool_failed` / `quota_blocked` 原始错误串）区分开。
const TURN_ERROR_PREFIX: &str = "turn_error:";

/// 定位 `agent.turn.outcome` 键并在其后的字节窗口内断言值字面量为 `error`。
///
/// 为什么需要窗口断言（Gate-1 A [BLOCKING] #3）：只断言「键存在」无法证伪「值写错」
///（例如值写成 `success` / `tool_failed`）；protobuf 里 key 与 value 相邻落盘，
/// 故取键后 16 字节窗口做值断言。窗口内还要求不出现 `success`，防止值被写成成功态。
fn assert_turn_outcome_value_is_error(wire: &str, context: &str) {
    let key = "agent.turn.outcome";
    let hits = wire.matches(key).count();
    assert_eq!(
        hits, 1,
        "PA154[{context}]：`{key}` 必须恰好出现一次（根 span 单值），实际 {hits} 次，body 片段：{}",
        truncate_for_message(wire)
    );
    let key_at = wire.find(key).expect("上一断言已保证键存在");
    let after = key_at + key.len();
    let window: String = wire[after..].chars().take(16).collect();
    assert!(
        window.contains("error"),
        "PA154[{context}]：`{key}` 的值必须是 `error`（键后 16 字节窗口 = `{window}`），body 片段：{}",
        truncate_for_message(wire)
    );
    assert!(
        !window.contains("success"),
        "PA154[{context}]：`{key}` 的值不得为 `success`（成功轮次必须完全不输出该键），窗口 = `{window}`"
    );
}

fn tool_input(tool_name: &str, status: &str, error: Option<&str>) -> ToolSpanInput {
    ToolSpanInput {
        tool_name: tool_name.to_string(),
        status: status.to_string(),
        duration_ms: Some(12),
        error: error.map(|text| text.to_string()),
    }
}

// ================================================== 既有常量漂移守卫（反向防过度删除）
//
// Gate-1 A 审查 [BLOCKING]：删除自指自证的字面量断言（`assert_eq!("x","x")` 恒真、零证伪能力）。
// 保留的只有**真实取值断言**：HEAD 已存在常量的值被改动即红（防 PA-154 顺手改名破坏子 span
// 与告警规则）。PA-154 新增键一律只在 wire 级断言（见下方 T1/T2 与 `assert_turn_outcome_error`），
// 不引用待新增符号（否则红相会退化成编译失败，违反契约 §0「红相纪律」）。

#[test]
fn wave45_existing_span_and_attribute_literals_are_frozen() {
    assert_eq!(otel::SPAN_AGENT_TURN, "agent.turn");
    assert_eq!(otel::SPAN_TOOL_CALL, "tool.call");
    assert_eq!(otel::SPAN_GOVERNANCE_DECIDE, "governance.decide");
    assert_eq!(otel::ATTR_AGENT_TURN_PHASE, "agent.turn.phase");
    assert_eq!(otel::ATTR_AGENT_TOOL_STATUS, "agent.tool.status");
}

// ============================================================== PA-153 资源归因
//
// F1（`Resource::builder()` 仅装配 service.name）⇒ wire 缺版本与环境归因（审计 P1-4）。

#[test]
fn pa153_t1_resource_carries_name_version_and_environment() {
    let _guard = gate_lock();

    // Arrange：本地回环管线 + 一个真实 turn（含 tool 子 span），确保确有 export。
    let sink = start_local_pipeline(2_000);
    {
        let mut turn = otel::begin_turn_trace("sess-pa153-t1", "turn-pa153-t1", 1);
        otel::record_tool_call_span(&tool_input("bash", "success", None));
        turn.set_phase("completed");
    }

    // Act + Assert
    let wire = drain_wire(sink);

    // 非空洞锚（契约 §1 T1 必含）：body 必须真的承载本轮 span 与 phase。
    assert!(
        wire.contains("agent.turn") && wire.contains("tool.call") && wire.contains("completed"),
        "wire body 必须承载 agent.turn + tool.call + phase=completed，实际 body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 1：resource 三键齐全（`service.name` 是既有项，作为计数基准）。
    assert!(
        wire.contains("service.name"),
        "PA153-T1 非空洞锚：resource 必须含 `service.name`，body 片段：{}",
        truncate_for_message(&wire)
    );
    assert!(
        wire.contains("service.version"),
        "F1/PA153-T1：resource 必须携带 `service.version`（当前实现只 with_service_name）。body 片段：{}",
        truncate_for_message(&wire)
    );
    assert!(
        wire.contains("deployment.environment"),
        "F1/PA153-T1：resource 必须携带 `deployment.environment`（不得恒缺）。body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 2：三键「各出现一次」（按同一 ResourceSpans 计数基准做相对比较，
    // 不假设导出被切分成几个 batch）。
    let name_hits = wire.matches("service.name").count();
    assert_eq!(
        wire.matches("service.version").count(),
        name_hits,
        "service.version 必须与 service.name 同基数出现（每个 ResourceSpans 各一次），body 片段：{}",
        truncate_for_message(&wire)
    );
    assert_eq!(
        wire.matches("deployment.environment").count(),
        name_hits,
        "deployment.environment 必须与 service.name 同基数出现（每个 ResourceSpans 各一次），body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 3：service.version 默认 = core crate 版本（测试 crate 与库同包 ⇒ CARGO_PKG_VERSION）。
    let expected_version = env!("CARGO_PKG_VERSION");
    assert!(
        wire.contains(expected_version),
        "PA153-T1：service.version 默认必须等于 crate 版本 `{expected_version}`，body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 4：debug 构建（cargo test）下环境必须为 `development`，不得硬编码 production
    //（审计缺陷正是「恒为 production」）；release 分支按契约由实现期纯函数单测覆盖。
    if cfg!(debug_assertions) {
        assert!(
            wire.contains("development"),
            "PA153-T1：debug 构建下 deployment.environment 必须为 `development`，body 片段：{}",
            truncate_for_message(&wire)
        );
    }
}

// ====================================================== PA-154 turn 根 span 错误承接
//
// F4（根 span 仅 phase=="failed" 置 ERROR）/ F5（工具失败谓词）⇒ 工具失败时根 span 仍
// `status={}`，按根状态告警全漏（审计 P1-1）。

#[test]
fn pa154_t1_failed_tool_span_marks_turn_error() {
    let _guard = gate_lock();

    // Arrange：一个 failed 的 tool span + phase completed（工具失败但轮次「正常收尾」——
    // 这正是最容易被漏报的组合）。
    let sink = start_local_pipeline(2_000);
    {
        let mut turn = otel::begin_turn_trace("sess-pa154-t1", "turn-pa154-t1", 4);
        otel::record_tool_call_span(&tool_input("bash", "failed", Some("boom")));
        turn.set_phase("completed");
    }

    // Act
    let wire = drain_wire(sink);

    // 非空洞锚：失败工具确实被采集（否则 `tool_failed` 缺席可以赖在「没采到 span」上）。
    assert!(
        wire.contains("boom"),
        "wire body 必须承载失败工具的 error 摘要 `boom`，body 片段：{}",
        truncate_for_message(&wire)
    );
    assert!(
        wire.contains("tool.call") && wire.contains("agent.turn"),
        "wire body 必须同时承载 tool.call 与 agent.turn，body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 1：根 span status.message 必须以统一前缀 `turn_error: tool_failed` 落盘
    //（Gate-1 A [BLOCKING] #1：单一前缀 + 归因标签，避免与子 span 既有文案混淆）。
    let expected = format!("{TURN_ERROR_PREFIX} tool_failed");
    assert!(
        wire.contains(&expected),
        "F4/F5/PA154-T1：子工具失败必须聚合到根 span 的 status.message=`turn_error: tool_failed`，body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 2：该前缀子串恰好出现 1 次 —— 子 span 现有文案（`failed` / `boom`）不得含 `turn_error`
    //（否则告警规则会把子 span 误判成根级聚合结论）。
    assert_eq!(
        wire.matches(TURN_ERROR_PREFIX).count(),
        1,
        "PA154-T1：`turn_error` 前缀只能出现在根 span status.message（恰好 1 次），body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 3：`agent.turn.outcome` 存在且**值 = error**（键存在不等于值正确）。
    assert_turn_outcome_value_is_error(&wire, "T1");
}

#[test]
fn pa154_t2_governance_block_marks_turn_error() {
    let _guard = gate_lock();

    // Arrange：治理 block 子 span + phase completed（F6：子 span 已置 quota_blocked，
    // 但根 span 仍不承接 ⇒ 根状态告警漏报）。
    let sink = start_local_pipeline(2_000);
    {
        let mut turn = otel::begin_turn_trace("sess-pa154-t2", "turn-pa154-t2", 5);
        let mut governance = otel::start_governance_decide_span();
        governance.record_verdict("session", "block", Some("hard_limit_reached"), "v1", 0);
        drop(governance);
        turn.set_phase("completed");
    }

    // Act
    let wire = drain_wire(sink);

    // 非空洞锚：治理子 span 确实被采集。
    assert!(
        wire.contains("governance.decide"),
        "wire body 必须承载 governance.decide 子 span，body 片段：{}",
        truncate_for_message(&wire)
    );
    assert!(
        wire.contains("hard_limit_reached"),
        "wire body 必须承载治理原因 `hard_limit_reached`，body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 1：根 span 承接治理 block ⇒ status.message 以统一前缀落盘
    //（Gate-1 A [BLOCKING] #2：必须带 `turn_error: ` 前缀，避免与 HEAD 既有 governance 子 span
    // 上的 `quota_blocked`（F6，子 span status.message）混淆）。
    let expected = format!("{TURN_ERROR_PREFIX} quota_blocked");
    assert!(
        wire.contains(&expected),
        "F4/F6/PA154-T2：治理 block 必须聚合到根 span 的 status.message=`turn_error: quota_blocked`，body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 2：`turn_error` 前缀恰好 1 次（子 span 的 `quota_blocked` 不带前缀 ⇒ 不混淆）。
    assert_eq!(
        wire.matches(TURN_ERROR_PREFIX).count(),
        1,
        "PA154-T2：`turn_error` 前缀只能出现在根 span（恰好 1 次），body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 3：`agent.turn.outcome` 存在且值 = error。
    assert_turn_outcome_value_is_error(&wire, "T2");
}

#[test]
fn pa154_t3_successful_turn_does_not_emit_failure_signals() {
    let _guard = gate_lock();

    // Arrange：全部成功的 tool span + phase completed（无 block）。
    let sink = start_local_pipeline(2_000);
    {
        let mut turn = otel::begin_turn_trace("sess-pa154-t3", "turn-pa154-t3", 6);
        otel::record_tool_call_span(&tool_input("read_file", "success", None));
        otel::record_tool_call_span(&tool_input("bash", "success", None));
        turn.set_phase("completed");
    }

    // Act
    let wire = drain_wire(sink);

    // 非空洞锚（契约 §2 T3 必含）：必须真的承载根 span + phase 键 + completed。
    assert!(
        wire.contains("agent.turn"),
        "PA154-T3 非空洞锚：body 必须承载 agent.turn，片段：{}",
        truncate_for_message(&wire)
    );
    assert!(
        wire.contains("agent.turn.phase") && wire.contains("completed"),
        "PA154-T3 非空洞锚：body 必须承载 agent.turn.phase=completed，片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 1：全成功轮次不得输出根级失败聚合键（保持「有 = 有错」的单义语义）。
    assert!(
        !wire.contains("agent.turn.outcome"),
        "PA154-T3：全成功 + phase completed 时**不得**输出 `agent.turn.outcome`（假阳性会污染失败率看板），body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 2：不得出现统一失败前缀，也不得出现任一归因标签（三者皆不出现）。
    assert!(
        !wire.contains("turn_error"),
        "PA154-T3：全成功时不得出现 `turn_error` 前缀，body 片段：{}",
        truncate_for_message(&wire)
    );
    assert!(
        !wire.contains("tool_failed"),
        "PA154-T3：全成功时不得出现 `tool_failed` 归因，body 片段：{}",
        truncate_for_message(&wire)
    );
    assert!(
        !wire.contains("quota_blocked"),
        "PA154-T3：无治理 block 时不得出现 `quota_blocked` 归因，body 片段：{}",
        truncate_for_message(&wire)
    );
}

/// PA154-ADV（对抗）：无 child error 但 `phase="failed"` —— 既有语义零变更 + 不得 panic。
///
/// 覆盖 prompt 冻结的对抗要求（契约 §2「语义边界」：phase 语义零变更、聚合 Drop 后清零）：
/// 新增的 outcome 聚合路径在「没有子 span 错误可聚合」时**不得** panic、不得虚构工具归因。
#[test]
fn pa154_adv_failed_phase_without_child_error_keeps_semantics() {
    let _guard = gate_lock();

    // Arrange：无任何 tool / governance span，仅 phase=failed。
    // 旧实现：`TurnTraceGuard::drop` 在 phase=="failed" 时 set_status(error("failed"))。
    let sink = start_local_pipeline(2_000);
    {
        let mut turn = otel::begin_turn_trace("sess-pa154-adv", "turn-pa154-adv", 7);
        turn.set_phase("failed");
        // drop 在此发生：若新增聚合路径在「无 child error」时 unwrap/panic，本用例直接失败。
    }

    // Act
    let wire = drain_wire(sink);

    // Assert 1：phase 语义优先 —— body 含 `failed`（既有 status.message 行为零变更）。
    assert!(
        wire.contains("failed"),
        "PA154-ADV：phase=failed 的既有语义必须保持（root status.message 含 `failed`），body 片段：{}",
        truncate_for_message(&wire)
    );
    assert!(
        wire.contains("agent.turn") && wire.contains("agent.turn.phase"),
        "PA154-ADV：本轮 span 与 phase 键必须已导出（否则 status 断言不可信），body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 2（Gate-1 A [BLOCKING] #4）：无子 span 错误 ⇒ **不得**出现统一聚合前缀
    //（`turn_error` 只属于「聚合到子 span 错误」的路径，phase 优先）。
    assert!(
        !wire.contains("turn_error"),
        "PA154-ADV：无 child error 时不得出现 `turn_error` 前缀（phase 优先），body 片段：{}",
        truncate_for_message(&wire)
    );

    // Assert 3：根 span 仍必须输出 `agent.turn.outcome` 且值 = error（phase=failed 也是错误轮次）。
    assert_turn_outcome_value_is_error(&wire, "ADV");

    // Assert 4：本轮没有任何工具失败，不得虚构工具失败归因。
    assert!(
        !wire.contains("tool_failed"),
        "PA154-ADV：本轮无 tool span 时不得虚构 `tool_failed` 归因，body 片段：{}",
        truncate_for_message(&wire)
    );
    assert!(
        !wire.contains("tool.call") && !wire.contains("quota_blocked"),
        "PA154-ADV 前置校验：本轮不应有 tool.call / quota_blocked 子 span，body 片段：{}",
        truncate_for_message(&wire)
    );
}

// ------------------------------------------------------------------ 诊断小工具

/// 断言失败时的 body 片段截断（避免把整包 protobuf 吐到 CI 日志里）。
fn truncate_for_message(wire: &str) -> String {
    const LIMIT: usize = 600;
    let printable: String = wire
        .chars()
        .map(|character| {
            if character.is_ascii_graphic() || character == ' ' {
                character
            } else {
                '.'
            }
        })
        .collect();
    if printable.chars().count() <= LIMIT {
        return printable;
    }
    let head: String = printable.chars().take(LIMIT).collect();
    format!("{head}…(共 {} 字符，已截断)", printable.chars().count())
}
