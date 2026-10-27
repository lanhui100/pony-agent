//! wave-54 冻结验收测试 · 只读 · 依据 `.dev-team/contract-matrix-wave54.md`
//!
//! 归属：**Test Agent 独占写域**（独立测试分权）。契约来源：
//! - `.dev-team/contract-matrix-wave54.md` §1 冻结契约 C1 / C2 / C5
//! - `.dev-team/contract-matrix-wave54.md` §2 验收用例 T1 / T2 / T3
//! - 驱动事实：真实 trace `eed9ba47…`（20 次 `llm.chat`，98.47s）；
//!   `docs/audit/observability-audit-2026-10-27b.md`
//!
//! 本文件一经冻结即**只读**：Executor 不得修改任何断言、不得删除用例、不得 `#[ignore]`。
//! 红相（stub `unimplemented!()` panic / 断言不满足）是预期状态，
//! 不允许为了让测试变绿而放宽断言。
//!
//! 覆盖（零外网；全部走回环 OTLP sink 的线级（wire）解码，或纯折叠值级断言）：
//! - **T1**（C1）`llm.chat` 四键真值：
//!   - stream（有 ttft）：`llm.call.duration_ms` / `llm.call.ttft_ms` /
//!     `llm.call.generation_ms` / `agent.usage.origin` 四键同时存在，
//!     且 `duration == ttft + generation`、`generation == duration - ttft`；
//!   - buffered（无 ttft）：**无** `llm.call.ttft_ms` 键，且 `generation == duration`；
//!   - `ttft > duration`（TTFT 不可信）：`llm.call.ttft_ms` 仍按 `Some` 原样输出，
//!     但 `generation == duration`（**不**抵扣、**不**产出负值）；
//!   - 未记录 usage 来源（`record_usage_origin(None)`）：**无** `agent.usage.origin` 键。
//!   每条「键不存在」断言都配同 span 的正向对照键（证明 span 确实导出了）。
//! - **T2**（C2/C4）根 `agent.turn` 速度唯一口径：
//!   `agent.turn.speed.tokens_per_second == Σout / (Σgen / 1000)`（±1e-6，
//!   Σ 由**同 trace 的 `llm.chat` 线级属性**反解，而非复用本地常量 ⇒ 两侧任一处口径漂移都会红）、
//!   `agent.turn.speed.basis == "turn_weighted"`、
//!   `agent.turn.duration_ms` / `agent.turn.ttft_ms` / `agent.turn.calls` 存在；
//!   有窗口能力 ⇒ `gen_ai.usage.context_window` + `context_ratio ≈ in/win` 存在；
//!   无窗口能力 ⇒ 两键**都不存在**（禁止 0 占位）。
//!   另含非正/NaN 速度不得落 wire（`agent.turn.speed.tokens_per_second` 缺键）。
//! - **T3**（C5）折叠不覆盖：
//!   `fold_all::<TraceProjectionState>` 给 step0(dur=500)、step1(无 dur) + `TurnEnd{1200}`
//!   ⇒ step0 仍 500、step1 == 1200；`settle_timeline` 不得用整轮值覆盖已有 per-call 值
//!   （单调用轮 dur=500 + TurnEnd{1200} ⇒ 仍 500）；
//!   `MetricsProjectionState` 轮级 `first_token_latency_ms` first-wins（后续 None 不冲刷）。
//!
//! 手法与断言纪律（与 `observability_reporting_wave53_test.rs` 同构）：
//! 1. 零外网：回环 OTLP sink + 极简 protobuf wire 解码（不新增依赖、不进 `src/`）；
//! 2. 属性键一律**字面量**断言（避免「常量被改后自指自证」），另设常量字面量冻结用例；
//! 3. 禁止 `assert!(true)` 式同义反复、禁止 `#[ignore]`、禁止吞异常、禁止 skip、禁止 sleep 轮询
//!    （仅 sink 线程用固定轮询等待 accept，硬截止 1.0–2.5s）；
//! 4. 全局门禁 / 全局 trace provider 是进程级单例 ⇒ 凡触达它们的用例一律在 `gate_lock()`
//!    串行下执行，每条用例结束 `otel::shutdown()` + 关闭门禁复位。

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::otel::{
    self, ATTR_AGENT_TURN_CALLS, ATTR_AGENT_TURN_DURATION_MS, ATTR_AGENT_TURN_SPEED_BASIS,
    ATTR_AGENT_TURN_TTFT_MS, ATTR_AGENT_USAGE_ORIGIN, ATTR_LLM_CALL_DURATION_MS,
    ATTR_LLM_CALL_GENERATION_MS, ATTR_LLM_CALL_TTFT_MS, DEFAULT_SAMPLING_RATIO,
};
use pony_agent_core::agent::projection::{fold_all, MetricsProjectionState, TraceProjectionState};
use pony_agent_core::agent::provider::TokenUsage;
use pony_agent_core::agent::telemetry::{ProviderLatencyKind, ProviderRequestKind};
use pony_agent_core::agent::turn_event::{TurnEndReason, TurnEvent};

// ---------------------------------------------------------------- 门禁串行锁
//
// 全局门禁是进程级 `AtomicBool`、全局 trace provider 是进程级单例，`cargo test` 默认多线程并发。
// 凡触达门禁 / 全局 provider 的用例必须持锁串行，并在结束处复位（与 wave-51/53 冻结文件同构）。

static GATE_LOCK: Mutex<()> = Mutex::new(());

fn gate_lock() -> MutexGuard<'static, ()> {
    GATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

// -------------------------------------------------- 本地 OTLP/HTTP sink（回环，零外网）
//
// 非阻塞 accept + 固定轮询 + 硬截止（禁止无界等待）：捕获 method/path/body 后回
// `200 + 空 ExportTraceServiceResponse`。硬截止保证负向断言（键缺失）不会悬空等待。

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

/// 回环端点配置（4 字段；本波**不**给 `OtelConfig` 加字段）。
fn local_config(port: u16) -> otel::OtelConfig {
    otel::OtelConfig {
        endpoint: format!("http://127.0.0.1:{port}"),
        service_name: "pony-agent-wave54-acceptance".to_string(),
        sampling_ratio: DEFAULT_SAMPLING_RATIO,
        headers: Vec::new(),
    }
}

// ------------------------------------------------- 极简 protobuf wire 解码器（仅测试内）
//
// 契约要求「线级解码该 span」，因此必须把属性真值反解出来（键名 + 类型 + 数值），而不是只做
// 整段 body 的 `contains` —— 后者会被同 trace 其它 span 的键满足，属不可证伪的假绿。
// 字段号已对照 opentelemetry-proto `trace/v1/trace.proto` + `common/v1/common.proto`：
//   ExportTraceServiceRequest.resource_spans = 1
//   ResourceSpans.scope_spans = 2 / ScopeSpans.spans = 2
//   Span.name = 5 / Span.attributes = 9
//   KeyValue.key = 1 / KeyValue.value = 2
//   AnyValue.string_value = 1 / bool_value = 2 / int_value = 3 / double_value = 4

enum WireValue {
    Varint(u64),
    /// fixed64 的 8 字节原样（`double_value` 需要 `from_le_bytes` 反解）。
    Fixed64([u8; 8]),
    Fixed32,
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
enum DecodedValue {
    Int(i64),
    Str(String),
    Bool(bool),
    Double(f64),
    Other,
}

#[derive(Debug)]
struct DecodedSpan {
    name: String,
    attributes: Vec<(String, DecodedValue)>,
}

fn read_varint(bytes: &[u8], cursor: &mut usize) -> u64 {
    let mut result: u64 = 0;
    let mut shift: u32 = 0;
    while *cursor < bytes.len() && shift < 64 {
        let byte = bytes[*cursor];
        *cursor += 1;
        result |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    result
}

fn parse_fields(bytes: &[u8]) -> Vec<(u64, WireValue)> {
    let mut fields: Vec<(u64, WireValue)> = Vec::new();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        let key = read_varint(bytes, &mut cursor);
        let number = key >> 3;
        match key & 0x7 {
            0 => {
                let value = read_varint(bytes, &mut cursor);
                fields.push((number, WireValue::Varint(value)));
            }
            1 => {
                let end = cursor.saturating_add(8);
                if end > bytes.len() {
                    break;
                }
                let mut fixed = [0u8; 8];
                fixed.copy_from_slice(&bytes[cursor..end]);
                cursor = end;
                fields.push((number, WireValue::Fixed64(fixed)));
            }
            2 => {
                let length = read_varint(bytes, &mut cursor) as usize;
                let end = cursor.saturating_add(length).min(bytes.len());
                fields.push((number, WireValue::Bytes(bytes[cursor..end].to_vec())));
                cursor = end;
            }
            5 => {
                cursor = cursor.saturating_add(4);
                fields.push((number, WireValue::Fixed32));
            }
            // 本波不出现 group（3/4），未知 wire type 直接停止解析，避免越界死循环。
            _ => break,
        }
    }
    fields
}

fn decode_any_value(bytes: &[u8]) -> DecodedValue {
    for (number, value) in parse_fields(bytes) {
        match (number, value) {
            (1, WireValue::Bytes(text)) => {
                return DecodedValue::Str(String::from_utf8_lossy(&text).to_string());
            }
            (2, WireValue::Varint(raw)) => return DecodedValue::Bool(raw != 0),
            (3, WireValue::Varint(raw)) => return DecodedValue::Int(raw as i64),
            (4, WireValue::Fixed64(raw)) => return DecodedValue::Double(f64::from_le_bytes(raw)),
            _ => {}
        }
    }
    DecodedValue::Other
}

fn decode_spans(body: &[u8]) -> Vec<DecodedSpan> {
    let mut spans: Vec<DecodedSpan> = Vec::new();
    for (number, resource_spans) in parse_fields(body) {
        let WireValue::Bytes(resource_spans) = resource_spans else {
            continue;
        };
        if number != 1 {
            continue;
        }
        for (number, scope_spans) in parse_fields(&resource_spans) {
            let WireValue::Bytes(scope_spans) = scope_spans else {
                continue;
            };
            if number != 2 {
                continue;
            }
            for (number, span) in parse_fields(&scope_spans) {
                let WireValue::Bytes(span) = span else {
                    continue;
                };
                if number != 2 {
                    continue;
                }
                let mut name = String::new();
                let mut attributes: Vec<(String, DecodedValue)> = Vec::new();
                for (number, field) in parse_fields(&span) {
                    let WireValue::Bytes(payload) = field else {
                        continue;
                    };
                    match number {
                        5 => name = String::from_utf8_lossy(&payload).to_string(),
                        9 => {
                            let mut key = String::new();
                            let mut value = DecodedValue::Other;
                            for (number, field) in parse_fields(&payload) {
                                let WireValue::Bytes(payload) = field else {
                                    continue;
                                };
                                match number {
                                    1 => key = String::from_utf8_lossy(&payload).to_string(),
                                    2 => value = decode_any_value(&payload),
                                    _ => {}
                                }
                            }
                            attributes.push((key, value));
                        }
                        _ => {}
                    }
                }
                spans.push(DecodedSpan { name, attributes });
            }
        }
    }
    spans
}

fn decoded_spans(captured: &[CapturedRequest]) -> Vec<DecodedSpan> {
    let mut spans: Vec<DecodedSpan> = Vec::new();
    for request in captured {
        spans.extend(decode_spans(&request.body));
    }
    spans
}

fn describe(captured: &[CapturedRequest]) -> Vec<(String, String)> {
    captured
        .iter()
        .map(|request| (request.method.clone(), request.path.clone()))
        .collect()
}

/// 一次完整导出周期：复位 → 回环 sink → `init` → 开门禁 → 造 span → `force_flush` → `shutdown`
/// → 解码。调用方必须持 `gate_lock`。
fn run_export<F: FnOnce()>(budget: Duration, body: F) -> Vec<DecodedSpan> {
    otel::set_reporting_enabled(false);
    otel::shutdown();

    let (port, sink) = spawn_otlp_sink(budget);
    let assembled = otel::init(local_config(port));
    assert!(
        assembled.is_ok(),
        "wave-54：本地回环端点必须可装配（装配失败只降级不 panic）：{assembled:?}"
    );
    otel::set_reporting_enabled(true);

    body();

    let flushed = otel::force_flush(3_000);
    assert!(
        flushed.is_ok(),
        "wave-54：已装配时 force_flush 必须成功（否则下述缺键断言可能只是「没导出」）：{flushed:?}"
    );
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(
        !captured.is_empty(),
        "wave-54：必须至少导出一次 POST /v1/traces，否则线级断言无意义（捕获：{:?}）",
        describe(&captured)
    );
    decoded_spans(&captured)
}

// ---------------------------------------------------------------- 解码结果断言小工具

fn wire_attr<'a>(span: &'a DecodedSpan, key: &str) -> Option<&'a DecodedValue> {
    span.attributes
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value)
}

fn span_keys(span: &DecodedSpan) -> Vec<&str> {
    span.attributes
        .iter()
        .map(|(name, _)| name.as_str())
        .collect()
}

/// 字符串属性：键缺失 ⇒ `None`；类型不符 ⇒ panic（类型退化不得静默通过）。
fn wire_str<'a>(span: &'a DecodedSpan, key: &str) -> Option<&'a str> {
    match wire_attr(span, key) {
        None => None,
        Some(DecodedValue::Str(value)) => Some(value.as_str()),
        Some(other) => panic!(
            "span `{}` 的属性 `{key}` 必须以字符串上 wire，实际 {other:?}；属性全集：{:?}",
            span.name,
            span_keys(span)
        ),
    }
}

/// 浮点属性：wire 上必须是 `double_value`（写进 string_value 会让 Langfuse 数值面失效）。
fn wire_f64(span: &DecodedSpan, key: &str) -> Option<f64> {
    match wire_attr(span, key) {
        None => None,
        Some(DecodedValue::Double(value)) => Some(*value),
        Some(other) => panic!(
            "span `{}` 的属性 `{key}` 必须以浮点（double_value）上 wire，实际 {other:?}；属性全集：{:?}",
            span.name,
            span_keys(span)
        ),
    }
}

/// 整型属性：必须是 `int_value`（落成字符串会破坏看板数值过滤/对账）。
fn wire_i64(span: &DecodedSpan, key: &str) -> Option<i64> {
    match wire_attr(span, key) {
        None => None,
        Some(DecodedValue::Int(value)) => Some(*value),
        Some(other) => panic!(
            "span `{}` 的属性 `{key}` 必须以整型（int_value）上 wire，实际 {other:?}；属性全集：{:?}",
            span.name,
            span_keys(span)
        ),
    }
}

/// 断言键**不存在**（缺键 ≠ 0/空串占位）。
fn assert_key_absent(span: &DecodedSpan, key: &str) {
    assert!(
        wire_attr(span, key).is_none(),
        "span `{}` 不得输出 `{key}`（禁止 0/空串/false 占位），实际 {:?}；属性全集：{:?}",
        span.name,
        wire_attr(span, key),
        span_keys(span)
    );
}

/// 必填整型键（缺失 ⇒ 带属性全集 panic，便于定位「没导出」与「键名漂移」）。
fn require_i64(span: &DecodedSpan, key: &str, label: &str) -> i64 {
    wire_i64(span, key).unwrap_or_else(|| {
        panic!(
            "{label}（BLOCKING）：span `{}` 必须输出 `{key}`；属性全集：{:?}",
            span.name,
            span_keys(span)
        )
    })
}

/// 必填字符串键。
fn require_str<'a>(span: &'a DecodedSpan, key: &str, label: &str) -> &'a str {
    wire_str(span, key).unwrap_or_else(|| {
        panic!(
            "{label}（BLOCKING）：span `{}` 必须输出 `{key}`；属性全集：{:?}",
            span.name,
            span_keys(span)
        )
    })
}

/// 按 span 名 + 某字符串属性值唯一定位 span（多命中/零命中都 panic，杜绝「模糊命中」）。
fn find_span_with<'a>(
    spans: &'a [DecodedSpan],
    name: &str,
    key: &str,
    expected: &str,
) -> &'a DecodedSpan {
    let matched: Vec<&DecodedSpan> = spans
        .iter()
        .filter(|span| span.name == name && wire_str(span, key) == Some(expected))
        .collect();
    match matched.as_slice() {
        [single] => single,
        other => panic!(
            "必须恰好 1 个 `{name}` span 满足 `{key}`==`{expected}`，实际 {} 个；导出 span 全量：{:?}",
            other.len(),
            spans
                .iter()
                .map(|span| (span.name.as_str(), span_keys(span)))
                .collect::<Vec<_>>()
        ),
    }
}

fn find_spans_named<'a>(spans: &'a [DecodedSpan], name: &str) -> Vec<&'a DecodedSpan> {
    spans.iter().filter(|span| span.name == name).collect()
}

/// 浮点近似断言（±1e-6；速度/比值不能用 `==` 比）。
fn assert_close(actual: f64, expected: f64, label: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= 1e-6,
        "{label}：期望 {expected}（±1e-6），实际 {actual}（偏差 {delta:e}）"
    );
}

// ==================================================== 常量字面量冻结（防自指自证）

/// C1/C2 新增属性键必须与 `.dev-team/contract-matrix-wave54.md` §1 逐字一致。
///
/// 若只断言「常量 == 常量」，把键名改成任意字符串都能过；因此此处钉**字面量**。
#[test]
fn t0_attribute_key_literals_frozen() {
    assert_eq!(
        ATTR_LLM_CALL_DURATION_MS, "llm.call.duration_ms",
        "C1：单次调用耗时键名必须逐字为 `llm.call.duration_ms`"
    );
    assert_eq!(
        ATTR_LLM_CALL_TTFT_MS, "llm.call.ttft_ms",
        "C1：首包耗时键名必须逐字为 `llm.call.ttft_ms`"
    );
    assert_eq!(
        ATTR_LLM_CALL_GENERATION_MS, "llm.call.generation_ms",
        "C1：生成耗时键名必须逐字为 `llm.call.generation_ms`"
    );
    assert_eq!(
        ATTR_AGENT_USAGE_ORIGIN, "agent.usage.origin",
        "C1/C3：usage 来源键名必须逐字为 `agent.usage.origin`"
    );
    assert_eq!(
        ATTR_AGENT_TURN_SPEED_BASIS, "agent.turn.speed.basis",
        "C2：速度口径标记键名必须逐字为 `agent.turn.speed.basis`"
    );
    assert_eq!(
        ATTR_AGENT_TURN_DURATION_MS, "agent.turn.duration_ms",
        "C2：turn 耗时键名必须逐字为 `agent.turn.duration_ms`"
    );
    assert_eq!(
        ATTR_AGENT_TURN_TTFT_MS, "agent.turn.ttft_ms",
        "C2：turn TTFT 键名必须逐字为 `agent.turn.ttft_ms`"
    );
    assert_eq!(
        ATTR_AGENT_TURN_CALLS, "agent.turn.calls",
        "C2：turn 已完成调用数键名必须逐字为 `agent.turn.calls`"
    );
}

// ============================================================ 键名常量（断言用字面量）

const KEY_LLM_DURATION: &str = "llm.call.duration_ms";
const KEY_LLM_TTFT: &str = "llm.call.ttft_ms";
const KEY_LLM_GENERATION: &str = "llm.call.generation_ms";
const KEY_USAGE_ORIGIN: &str = "agent.usage.origin";
const KEY_TURN_SPEED_BASIS: &str = "agent.turn.speed.basis";
const KEY_TURN_DURATION: &str = "agent.turn.duration_ms";
const KEY_TURN_TTFT: &str = "agent.turn.ttft_ms";
const KEY_TURN_CALLS: &str = "agent.turn.calls";
const KEY_TURN_SPEED: &str = "agent.turn.speed.tokens_per_second";
const KEY_CALL_SPEED: &str = "gen_ai.speed.tokens_per_second";
const KEY_CONTEXT_WINDOW: &str = "gen_ai.usage.context_window";
const KEY_CONTEXT_RATIO: &str = "gen_ai.usage.context_ratio";
const KEY_GEN_AI_INPUT: &str = "gen_ai.usage.input_tokens";
const KEY_GEN_AI_OUTPUT: &str = "gen_ai.usage.output_tokens";
const KEY_REQUEST_MODEL: &str = "gen_ai.request.model";
const KEY_TURN_ID: &str = "agent.turn.id";

// ============================================================ T1 · llm.chat 四键真值

const T1_MODEL_STREAM: &str = "wave54-t1-stream";
const T1_MODEL_BUFFERED: &str = "wave54-t1-buffered";
const T1_MODEL_TTFT_OVER_DUR: &str = "wave54-t1-ttft-over-dur";
const T1_MODEL_NO_ORIGIN: &str = "wave54-t1-no-origin";

const T1_STREAM_DURATION_MS: i64 = 1_200;
const T1_STREAM_TTFT_MS: i64 = 200;
const T1_STREAM_GENERATION_MS: i64 = 1_000;
const T1_BUFFERED_DURATION_MS: i64 = 900;
/// TTFT 不可信档：ttft(800) > duration(500) ⇒ 不抵扣，generation == duration == 500。
const T1_UNTRUSTED_DURATION_MS: i64 = 500;
const T1_UNTRUSTED_TTFT_MS: i64 = 800;

/// T1（C1）：`llm.chat` 的 duration / ttft / generation / origin 四键真值。
///
/// 关键判据：
/// - stream（有 ttft）：`duration == ttft + generation`；
/// - buffered（无 ttft）：**无** `llm.call.ttft_ms` 键，且 `generation == duration`；
/// - `ttft > duration`：`llm.call.ttft_ms` 原样输出（`Some` 才输出），`generation == duration`；
/// - 未记录 usage 来源：**无** `agent.usage.origin` 键。
#[test]
fn t1_llm_chat_call_timing_wire() {
    let _guard = gate_lock();

    let spans = run_export(Duration::from_millis(2_500), || {
        let _turn = otel::begin_turn_trace("sess-wave54-t1", "turn-wave54-t1", 1);

        // ① stream 调用：有首包（ttft）。
        let mut stream = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T1_MODEL_STREAM),
            "chat",
        );
        stream.record_usage(Some(1_000), Some(100));
        stream.record_call_timing(
            Some(T1_STREAM_DURATION_MS as u64),
            Some(T1_STREAM_TTFT_MS as u64),
        );
        stream.record_usage_origin(Some("provider"));
        drop(stream);

        // ② buffered 调用：无首包（ttft 缺失）。
        let mut buffered = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T1_MODEL_BUFFERED),
            "chat",
        );
        buffered.record_usage(Some(800), Some(90));
        buffered.record_call_timing(Some(T1_BUFFERED_DURATION_MS as u64), None);
        buffered.record_usage_origin(Some("estimated_chars"));
        drop(buffered);

        // ③ TTFT 不可信（ttft > duration）：不得抵扣、不得产出负 generation。
        let mut untrusted = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T1_MODEL_TTFT_OVER_DUR),
            "chat",
        );
        untrusted.record_usage(Some(700), Some(40));
        untrusted.record_call_timing(
            Some(T1_UNTRUSTED_DURATION_MS as u64),
            Some(T1_UNTRUSTED_TTFT_MS as u64),
        );
        untrusted.record_usage_origin(Some("provider"));
        drop(untrusted);

        // ④ 未记录 usage 来源：origin 键必须不存在（未知 ≠ 某默认值）。
        let mut no_origin = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T1_MODEL_NO_ORIGIN),
            "chat",
        );
        no_origin.record_usage(Some(600), Some(30));
        no_origin.record_call_timing(Some(700), Some(100));
        no_origin.record_usage_origin(None);
        drop(no_origin);
    });

    let chats = find_spans_named(&spans, "llm.chat");
    assert_eq!(
        chats.len(),
        4,
        "T1：本用例必须导出恰好 4 个 `llm.chat` span，实际 {}；导出 span 全量：{:?}",
        chats.len(),
        spans
            .iter()
            .map(|span| (span.name.as_str(), span_keys(span)))
            .collect::<Vec<_>>()
    );

    // ---------- ① stream：四键齐备且 duration == ttft + generation ----------
    let stream = find_span_with(&spans, "llm.chat", KEY_REQUEST_MODEL, T1_MODEL_STREAM);
    assert_eq!(
        wire_i64(stream, KEY_GEN_AI_INPUT),
        Some(1_000),
        "T1①：正向对照——stream span 必须解出既有 `gen_ai.usage.input_tokens`（否则「键缺失」断言无意义）；属性全集：{:?}",
        span_keys(stream)
    );
    let stream_duration = require_i64(stream, KEY_LLM_DURATION, "T1①（C1）");
    let stream_ttft = require_i64(stream, KEY_LLM_TTFT, "T1①（C1）");
    let stream_generation = require_i64(stream, KEY_LLM_GENERATION, "T1①（C1）");
    assert_eq!(
        stream_duration, T1_STREAM_DURATION_MS,
        "T1①：`llm.call.duration_ms` 必须等于该次调用耗时 {T1_STREAM_DURATION_MS}ms"
    );
    assert_eq!(
        stream_ttft, T1_STREAM_TTFT_MS,
        "T1①：`llm.call.ttft_ms` 必须等于入参首包耗时 {T1_STREAM_TTFT_MS}ms"
    );
    assert_eq!(
        stream_generation, T1_STREAM_GENERATION_MS,
        "T1①：`llm.call.generation_ms` 必须等于 duration - ttft（{T1_STREAM_GENERATION_MS}ms）"
    );
    assert_eq!(
        stream_duration,
        stream_ttft + stream_generation,
        "T1①（C1 核心）：`duration == ttft + generation`，实际 {stream_duration} != {stream_ttft} + {stream_generation}"
    );
    assert_eq!(
        require_str(stream, KEY_USAGE_ORIGIN, "T1①（C3）"),
        "provider",
        "T1①（C3）：provider 解析路径必须以 `provider` 上 wire（估算值不得冒充真值）"
    );

    // ---------- ② buffered：无 ttft 键，generation == duration ----------
    let buffered = find_span_with(&spans, "llm.chat", KEY_REQUEST_MODEL, T1_MODEL_BUFFERED);
    assert_eq!(
        wire_i64(buffered, KEY_GEN_AI_OUTPUT),
        Some(90),
        "T1②：正向对照——buffered span 必须解出既有 `gen_ai.usage.output_tokens`；属性全集：{:?}",
        span_keys(buffered)
    );
    let buffered_duration = require_i64(buffered, KEY_LLM_DURATION, "T1②（C1）");
    let buffered_generation = require_i64(buffered, KEY_LLM_GENERATION, "T1②（C1）");
    assert_eq!(
        buffered_duration, T1_BUFFERED_DURATION_MS,
        "T1②：`llm.call.duration_ms` 必须等于 {T1_BUFFERED_DURATION_MS}ms"
    );
    assert_eq!(
        buffered_generation, buffered_duration,
        "T1②（C1）：无首包时 `generation == duration`，实际 gen={buffered_generation} dur={buffered_duration}"
    );
    assert_key_absent(buffered, KEY_LLM_TTFT);
    assert_eq!(
        require_str(buffered, KEY_USAGE_ORIGIN, "T1②（C3）"),
        "estimated_chars",
        "T1②（C3）：估算路径必须以 `estimated_chars` 上 wire"
    );

    // ---------- ③ ttft > duration：不抵扣、非负 ----------
    let untrusted = find_span_with(
        &spans,
        "llm.chat",
        KEY_REQUEST_MODEL,
        T1_MODEL_TTFT_OVER_DUR,
    );
    assert_eq!(
        wire_i64(untrusted, KEY_GEN_AI_INPUT),
        Some(700),
        "T1③：正向对照——必须解出既有 `gen_ai.usage.input_tokens`；属性全集：{:?}",
        span_keys(untrusted)
    );
    let untrusted_duration = require_i64(untrusted, KEY_LLM_DURATION, "T1③（C1）");
    let untrusted_ttft = require_i64(untrusted, KEY_LLM_TTFT, "T1③（C1）");
    let untrusted_generation = require_i64(untrusted, KEY_LLM_GENERATION, "T1③（C1）");
    assert_eq!(
        untrusted_ttft, T1_UNTRUSTED_TTFT_MS,
        "T1③（C1）：`llm.call.ttft_ms` 语义是「仅 Some 时输出」，不可信值也必须原样上 wire"
    );
    assert_eq!(
        untrusted_duration, T1_UNTRUSTED_DURATION_MS,
        "T1③：`llm.call.duration_ms` 必须等于 {T1_UNTRUSTED_DURATION_MS}ms"
    );
    assert_eq!(
        untrusted_generation, untrusted_duration,
        "T1③（C1）：`duration > ttft` 不成立时 `generation == duration`（禁止负值），实际 gen={untrusted_generation} dur={untrusted_duration}"
    );
    assert!(
        untrusted_generation >= 0,
        "T1③：generation 不得为负（i64 语义下必须是 u64 安全映射），实际 {untrusted_generation}"
    );

    // ---------- ④ 未记录 usage 来源：origin 键不存在 ----------
    let no_origin = find_span_with(&spans, "llm.chat", KEY_REQUEST_MODEL, T1_MODEL_NO_ORIGIN);
    assert_eq!(
        wire_i64(no_origin, KEY_GEN_AI_INPUT),
        Some(600),
        "T1④：正向对照——必须解出既有 `gen_ai.usage.input_tokens`（证明 span 已导出）；属性全集：{:?}",
        span_keys(no_origin)
    );
    assert_eq!(
        require_i64(no_origin, KEY_LLM_DURATION, "T1④（C1）"),
        700,
        "T1④：正向对照——`llm.call.duration_ms` 必须存在（与「origin 缺键」形成正反对照）"
    );
    assert_key_absent(no_origin, KEY_USAGE_ORIGIN);
}

// ============================================================ T2 · 根 span 速度唯一口径

const T2_MODEL_FIRST: &str = "wave54-t2-call-1";
const T2_MODEL_SECOND: &str = "wave54-t2-call-2";
const T2_MODEL_NO_WINDOW: &str = "wave54-t2-no-window";

const T2_FIRST_OUTPUT: f64 = 100.0;
const T2_SECOND_OUTPUT: f64 = 200.0;
const T2_FIRST_GENERATION_MS: f64 = 1_000.0;
const T2_SECOND_GENERATION_MS: f64 = 1_800.0;
const T2_WINDOW_TOKENS: i64 = 10_000;
const T2_WINDOW_INPUT_TOKENS: i64 = 1_000;

/// T2（C2/C4）：根 `agent.turn` 的加权生成速度必须等于 `Σout / (Σgen / 1000)`，
/// 其中 Σ 由**同 trace 的 `llm.chat` 线级属性**反解（cross-span 一致，非本地常量自证）。
#[test]
fn t2_turn_weighted_speed_wire() {
    let _guard = gate_lock();

    // 两次调用的真值（与下方 record_* 入参一一对应）：
    //   call1: out=100, dur=1200, ttft=200 ⇒ gen=1000
    //   call2: out=200, dur=1800, 无 ttft   ⇒ gen=1800
    // ⇒ Σout=300, Σgen=2800ms ⇒ speed = 300 / 2.8 = 107.142857… t/s
    let expected_speed = (T2_FIRST_OUTPUT + T2_SECOND_OUTPUT)
        / ((T2_FIRST_GENERATION_MS + T2_SECOND_GENERATION_MS) / 1_000.0);
    let turn_duration_ms = 3_000u64; // per-call 耗时之和（本用例最小 turn 装配）
    let turn_ttft_ms = 200u64; // 首个带 ttft 的 per-call 值
    let turn_calls = 2u32;

    let spans = run_export(Duration::from_millis(2_500), || {
        let mut turn = otel::begin_turn_trace("sess-wave54-t2", "turn-wave54-t2", 2);

        // call1：有 ttft + 有窗口能力。
        let mut first = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T2_MODEL_FIRST),
            "chat",
        );
        first.record_usage(Some(T2_WINDOW_INPUT_TOKENS as u64), Some(100));
        first.record_call_timing(Some(1_200), Some(200));
        first.record_usage_origin(Some("provider"));
        first.record_speed_and_context(Some(100.0), Some(T2_WINDOW_TOKENS as u64));
        drop(first);

        // call2：无 ttft + 无窗口能力（禁止 0 占位）。
        let mut second = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T2_MODEL_SECOND),
            "chat",
        );
        second.record_usage(Some(2_000), Some(200));
        second.record_call_timing(Some(1_800), None);
        second.record_usage_origin(Some("provider"));
        second.record_speed_and_context(Some(200.0 / 1.8), None);
        drop(second);

        // 根 span：速度口径 + turn 级真值。
        turn.record_turn_speed(Some(expected_speed));
        turn.record_turn_timing(
            Some(turn_duration_ms),
            Some(turn_ttft_ms),
            Some(turn_calls),
        );
    });

    let root = find_span_with(&spans, "agent.turn", KEY_TURN_ID, "turn-wave54-t2");

    // ---------- C2：速度 == Σout / (Σgen/1000)，Σ 从 llm.chat 线级属性反解 ----------
    let mut sum_output_tokens: f64 = 0.0;
    let mut sum_generation_ms: f64 = 0.0;
    for model in [T2_MODEL_FIRST, T2_MODEL_SECOND] {
        let chat = find_span_with(&spans, "llm.chat", KEY_REQUEST_MODEL, model);
        let duration = require_i64(chat, KEY_LLM_DURATION, "T2（C1）") as f64;
        let generation = require_i64(chat, KEY_LLM_GENERATION, "T2（C1）") as f64;
        let output = wire_i64(chat, KEY_GEN_AI_OUTPUT).unwrap_or_else(|| {
            panic!(
                "T2：`llm.chat`(model={model}) 必须解出既有 `gen_ai.usage.output_tokens`；属性全集：{:?}",
                span_keys(chat)
            )
        }) as f64;
        // 单次调用的 gen 必须与 duration/ttft 自洽（C1 与 C2 同一公式）。
        match wire_i64(chat, KEY_LLM_TTFT) {
            Some(ttft) if duration > ttft as f64 => assert_eq!(
                generation,
                duration - ttft as f64,
                "T2：`llm.chat`(model={model}) 的 generation 必须等于 duration - ttft"
            ),
            _ => assert_eq!(
                generation, duration,
                "T2：`llm.chat`(model={model}) 无 ttft 或 ttft>=dur 时 generation 必须等于 duration"
            ),
        }
        sum_output_tokens += output;
        sum_generation_ms += generation;
    }
    assert_eq!(
        sum_output_tokens, T2_FIRST_OUTPUT + T2_SECOND_OUTPUT,
        "T2：Σout 必须由两次调用线级 `gen_ai.usage.output_tokens` 相加得到（300）"
    );
    assert_eq!(
        sum_generation_ms,
        T2_FIRST_GENERATION_MS + T2_SECOND_GENERATION_MS,
        "T2：Σgen 必须由两次调用线级 `llm.call.generation_ms` 相加得到（2800ms）"
    );

    let wire_speed = wire_f64(root, KEY_TURN_SPEED).unwrap_or_else(|| {
        panic!(
            "T2（BLOCKING）：根 `agent.turn` 必须输出 `{KEY_TURN_SPEED}`；属性全集：{:?}",
            span_keys(root)
        )
    });
    let recomputed_speed = sum_output_tokens / (sum_generation_ms / 1_000.0);
    assert_close(
        wire_speed,
        recomputed_speed,
        &format!(
            "T2（C2 核心）：`{KEY_TURN_SPEED}` 必须等于 Σout/(Σgen/1000) = {recomputed_speed} t/s"
        ),
    );
    assert_close(
        wire_speed,
        expected_speed,
        "T2：加权生成速度必须等于 107.142857 t/s（非整轮吞吐 300/3.0=100）",
    );
    assert!(
        wire_speed.is_finite() && wire_speed > 0.0,
        "T2：速度必须是有限正值，实际 {wire_speed}"
    );

    // ---------- C2：basis / duration / ttft / calls ----------
    assert_eq!(
        require_str(root, KEY_TURN_SPEED_BASIS, "T2（C2）"),
        "turn_weighted",
        "T2（C2）：速度口径标记必须为 `turn_weighted`（单一口径）"
    );
    assert_eq!(
        require_i64(root, KEY_TURN_DURATION, "T2（C2）"),
        turn_duration_ms as i64,
        "T2（C2）：`agent.turn.duration_ms` 必须等于 turn 级耗时 {turn_duration_ms}ms"
    );
    assert_eq!(
        require_i64(root, KEY_TURN_TTFT, "T2（C2）"),
        turn_ttft_ms as i64,
        "T2（C2）：`agent.turn.ttft_ms` 必须是首个带 ttft 的 per-call 值 {turn_ttft_ms}ms"
    );
    assert_eq!(
        require_i64(root, KEY_TURN_CALLS, "T2（C2）"),
        turn_calls as i64,
        "T2（C2）：`agent.turn.calls` 必须是已完成调用数 {turn_calls}"
    );

    // ---------- C4：有窗口能力 ⇒ window + ratio ≈ in/win ----------
    let first = find_span_with(&spans, "llm.chat", KEY_REQUEST_MODEL, T2_MODEL_FIRST);
    assert_eq!(
        require_i64(first, KEY_CONTEXT_WINDOW, "T2（C4）"),
        T2_WINDOW_TOKENS,
        "T2（C4）：有窗口能力时必须输出 `gen_ai.usage.context_window` = {T2_WINDOW_TOKENS}"
    );
    let ratio = wire_f64(first, KEY_CONTEXT_RATIO).unwrap_or_else(|| {
        panic!(
            "T2（C4）（BLOCKING）：有窗口能力时必须输出 `{KEY_CONTEXT_RATIO}`；属性全集：{:?}",
            span_keys(first)
        )
    });
    let expected_ratio = T2_WINDOW_INPUT_TOKENS as f64 / T2_WINDOW_TOKENS as f64;
    assert_close(
        ratio,
        expected_ratio,
        "T2（C4）：`gen_ai.usage.context_ratio` 必须 ≈ in/win = 0.1",
    );

    // ---------- C4：无窗口能力 ⇒ 两键都不存在（禁止 0 占位） ----------
    let no_window = find_span_with(&spans, "llm.chat", KEY_REQUEST_MODEL, T2_MODEL_SECOND);
    assert!(
        wire_f64(no_window, KEY_CALL_SPEED).is_some(),
        "T2：正向对照——无窗口能力的 `llm.chat` 仍必须解出既有 `{KEY_CALL_SPEED}`；属性全集：{:?}",
        span_keys(no_window)
    );
    assert_key_absent(no_window, KEY_CONTEXT_WINDOW);
    assert_key_absent(no_window, KEY_CONTEXT_RATIO);
    assert!(T2_MODEL_NO_WINDOW.starts_with("wave54-t2"), "常量占位自检");
}

/// T2 补充（C2 守卫）：非正 / NaN / 无穷速度**不得**落到 `agent.turn.speed.tokens_per_second`
/// （无值即缺键，禁止写 0 / NaN 占位）。
#[test]
fn t2_turn_speed_not_recorded_when_not_finite_or_non_positive() {
    let _guard = gate_lock();

    let spans = run_export(Duration::from_millis(2_500), || {
        // ① 0 速度（Σgen 为 0 时的退化值）⇒ 不得写键。
        let mut zero = otel::begin_turn_trace("sess-wave54-t2b", "turn-wave54-t2b-zero", 1);
        zero.record_turn_speed(Some(0.0));
        drop(zero);

        // ② NaN ⇒ 不得写键。
        let mut nan = otel::begin_turn_trace("sess-wave54-t2b", "turn-wave54-t2b-nan", 2);
        nan.record_turn_speed(Some(f64::NAN));
        drop(nan);

        // ③ 无穷 ⇒ 不得写键。
        let mut infinite = otel::begin_turn_trace("sess-wave54-t2b", "turn-wave54-t2b-inf", 3);
        infinite.record_turn_speed(Some(f64::INFINITY));
        drop(infinite);

        // ④ 正向对照：有限正速度必须写键。
        let mut valid = otel::begin_turn_trace("sess-wave54-t2b", "turn-wave54-t2b-valid", 4);
        valid.record_turn_speed(Some(95.46));
        drop(valid);
    });

    for (turn_id, label) in [
        ("turn-wave54-t2b-zero", "0 速度"),
        ("turn-wave54-t2b-nan", "NaN 速度"),
        ("turn-wave54-t2b-inf", "无穷速度"),
    ] {
        let root = find_span_with(&spans, "agent.turn", KEY_TURN_ID, turn_id);
        assert_eq!(
            wire_str(root, KEY_TURN_ID),
            Some(turn_id),
            "T2b：正向对照——根 span 必须解出 `{KEY_TURN_ID}`（证明 span 已导出）；属性全集：{:?}",
            span_keys(root)
        );
        assert_key_absent(root, KEY_TURN_SPEED);
        assert!(
            wire_f64(root, KEY_TURN_SPEED).is_none(),
            "T2b（C2）：{label}不得上 wire，实际 {:?}",
            wire_attr(root, KEY_TURN_SPEED)
        );
    }

    let valid = find_span_with(&spans, "agent.turn", KEY_TURN_ID, "turn-wave54-t2b-valid");
    let valid_speed = wire_f64(valid, KEY_TURN_SPEED).unwrap_or_else(|| {
        panic!(
            "T2b：正向对照——有限正速度必须输出 `{KEY_TURN_SPEED}`；属性全集：{:?}",
            span_keys(valid)
        )
    });
    assert_close(valid_speed, 95.46, "T2b：有限正速度必须原样上 wire");
}

// ============================================================ T3 · 折叠不覆盖（C5）

const T3_TURN_ID: &str = "turn-wave54-t3";

fn usage(input: Option<u64>, output: Option<u64>) -> TokenUsage {
    // `..Default::default()`：C3 会向 `TokenUsage` 追加 `origin` 等字段，
    // 结构更新语法让本冻结用例对新字段免疫（不因实现新增字段而编译红）。
    TokenUsage {
        input_tokens: input,
        output_tokens: output,
        ..Default::default()
    }
}

fn provider_usage(
    turn_id: &str,
    step: u32,
    model: &str,
    first_token_latency_ms: Option<u64>,
    turn_duration_ms: Option<u64>,
) -> TurnEvent {
    TurnEvent::ProviderUsage {
        turn_id: turn_id.to_string(),
        step,
        request_kind: ProviderRequestKind::InitialRequest,
        usage: usage(Some(1_000), Some(100)),
        cache_hit_input_tokens: None,
        cache_miss_input_tokens: None,
        prefix_mutation_reasons: Vec::new(),
        first_token_latency_ms,
        turn_duration_ms,
        latency_kind: ProviderLatencyKind::ProviderStream,
        provider: "PonyLlm".to_string(),
        model: model.to_string(),
    }
}

/// T3（C5 §7.4）：`fold_all::<TraceProjectionState>` 折叠后，整轮 `turn_duration_ms`
/// 只能**补**缺失的 per-call 值，不得覆盖已有值。
#[test]
fn t3_trace_projection_keeps_per_call_duration() {
    let events: Vec<(u64, TurnEvent)> = vec![
        (
            1,
            TurnEvent::TurnStart {
                turn_id: T3_TURN_ID.to_string(),
            },
        ),
        (
            2,
            TurnEvent::StepStart {
                turn_id: T3_TURN_ID.to_string(),
                step: 0,
                first_token_latency_ms: Some(120),
            },
        ),
        // step0：per-call 耗时 500ms（真值）。
        (
            3,
            provider_usage(T3_TURN_ID, 0, "wave54-t3-step0", Some(120), Some(500)),
        ),
        (
            4,
            TurnEvent::StepStart {
                turn_id: T3_TURN_ID.to_string(),
                step: 1,
                first_token_latency_ms: None,
            },
        ),
        // step1：无 per-call 耗时（缺失，等待整轮值兜底）。
        (5, provider_usage(T3_TURN_ID, 1, "wave54-t3-step1", None, None)),
        (
            6,
            TurnEvent::TurnEnd {
                turn_id: T3_TURN_ID.to_string(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: Some(1_200),
            },
        ),
    ];

    let state: TraceProjectionState = fold_all::<TraceProjectionState, TraceProjectionState>(&events);
    let trace = state.trace_for_turn(T3_TURN_ID).unwrap_or_else(|| {
        panic!("T3：TurnEnd 后必须折叠出 turn `{T3_TURN_ID}` 的 trace（否则 timeline 断言无落点）")
    });

    let call_models: Vec<&pony_agent_core::agent::session::TraceTimelineEntry> = trace
        .trace_timeline
        .iter()
        .filter(|entry| entry.kind == "call_model")
        .collect();
    assert_eq!(
        call_models.len(),
        2,
        "T3：折叠后必须恰好有 2 个 call_model 条目（step0/step1），实际 {:?}",
        trace
            .trace_timeline
            .iter()
            .map(|entry| (entry.kind.as_str(), entry.provider_model.clone()))
            .collect::<Vec<_>>()
    );

    let step0 = call_models
        .iter()
        .find(|entry| entry.provider_model.as_deref() == Some("wave54-t3-step0"))
        .unwrap_or_else(|| {
            panic!(
                "T3：必须能按 provider_model 定位 step0 的 call_model 条目，实际 {:?}",
                call_models
                    .iter()
                    .map(|entry| entry.provider_model.clone())
                    .collect::<Vec<_>>()
            )
        });
    let step1 = call_models
        .iter()
        .find(|entry| entry.provider_model.as_deref() == Some("wave54-t3-step1"))
        .unwrap_or_else(|| {
            panic!(
                "T3：必须能按 provider_model 定位 step1 的 call_model 条目，实际 {:?}",
                call_models
                    .iter()
                    .map(|entry| entry.provider_model.clone())
                    .collect::<Vec<_>>()
            )
        });

    assert_eq!(
        step0.turn_duration_ms,
        Some(500),
        "T3（C5 核心）：step0 已有 per-call 耗时 500ms，整轮 1200ms 不得覆盖它"
    );
    assert_eq!(
        step0.first_token_latency_ms,
        Some(120),
        "T3：step0 的 per-call TTFT 必须保持 120ms"
    );
    assert_eq!(
        step1.turn_duration_ms,
        Some(1_200),
        "T3（C5）：step1 缺失 per-call 耗时 ⇒ 必须补整轮值 1200ms"
    );
    // 运行态产物与折叠产物同口径：turn 级字段仍是整轮值。
    assert_eq!(
        trace.turn_duration_ms,
        Some(1_200),
        "T3：turn trace 的 `turn_duration_ms` 必须是整轮值 1200ms"
    );
}

/// T3（C5 §7.4 反向对照）：**单调用轮**的 per-call 耗时已存在时，整轮值同样不得覆盖。
///
/// 这是 `settle_timeline` 现状缺陷的直接判据（当前实现对「末条 call_model」无条件赋值，
/// 单调用轮必然把 500ms 冲成 1200ms）。
#[test]
fn t3_settle_timeline_does_not_overwrite_existing_per_call_duration() {
    let events: Vec<(u64, TurnEvent)> = vec![
        (
            1,
            TurnEvent::TurnStart {
                turn_id: "turn-wave54-t3-single".to_string(),
            },
        ),
        (
            2,
            TurnEvent::StepStart {
                turn_id: "turn-wave54-t3-single".to_string(),
                step: 0,
                first_token_latency_ms: Some(90),
            },
        ),
        (
            3,
            provider_usage(
                "turn-wave54-t3-single",
                0,
                "wave54-t3-single",
                Some(90),
                Some(500),
            ),
        ),
        (
            4,
            TurnEvent::TurnEnd {
                turn_id: "turn-wave54-t3-single".to_string(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: Some(1_200),
            },
        ),
    ];

    let state: TraceProjectionState = fold_all::<TraceProjectionState, TraceProjectionState>(&events);
    let trace = state
        .trace_for_turn("turn-wave54-t3-single")
        .unwrap_or_else(|| panic!("T3：TurnEnd 后必须折叠出单调用轮的 trace"));
    assert_eq!(
        trace.trace_timeline.len(),
        1,
        "T3：单调用轮必须只有 1 个 timeline 条目，实际 {:?}",
        trace
            .trace_timeline
            .iter()
            .map(|entry| entry.kind.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        trace.trace_timeline[0].turn_duration_ms,
        Some(500),
        "T3（C5 核心）：单调用轮已有 per-call 耗时 500ms，整轮 1200ms 不得覆盖"
    );
    assert_eq!(
        trace.trace_timeline[0].first_token_latency_ms,
        Some(90),
        "T3：单调用轮的 per-call TTFT 必须保持 90ms"
    );
}

/// T3（C5）：`MetricsProjectionState` 轮级 `first_token_latency_ms` 必须 **first-wins**，
/// 后到的 `None` 不得冲刷已有值（与 `TraceProjection` 一致）。
#[test]
fn t3_metrics_first_token_latency_first_wins() {
    let events: Vec<(u64, TurnEvent)> = vec![
        (
            1,
            TurnEvent::TurnStart {
                turn_id: "turn-wave54-t3-metrics".to_string(),
            },
        ),
        // 首次调用：TTFT = 120ms（真值）。
        (
            2,
            provider_usage(
                "turn-wave54-t3-metrics",
                0,
                "wave54-t3-metrics-1",
                Some(120),
                Some(500),
            ),
        ),
        // 第二次调用：无 TTFT（None）——不得冲刷首次值。
        (
            3,
            provider_usage(
                "turn-wave54-t3-metrics",
                1,
                "wave54-t3-metrics-2",
                None,
                Some(700),
            ),
        ),
    ];

    let metrics: MetricsProjectionState =
        fold_all::<MetricsProjectionState, MetricsProjectionState>(&events);
    let aggregate = metrics.by_turn.get("turn-wave54-t3-metrics").unwrap_or_else(|| {
        panic!(
            "T3：MetricsProjection 必须产出 turn `turn-wave54-t3-metrics` 的聚合，实际 keys={:?}",
            metrics.by_turn.keys().collect::<Vec<_>>()
        )
    });
    assert_eq!(
        aggregate.first_token_latency_ms,
        Some(120),
        "T3（C5 核心）：轮级 `first_token_latency_ms` 必须 first-wins（首次 120ms 不被后续 None 冲刷）"
    );
    // 正向对照：聚合其它字段仍按 addReplacing 累计，证明本用例的数据确实折叠进来了。
    assert_eq!(
        aggregate.output_tokens, 200,
        "T3：正向对照——两次调用 output_tokens 必须聚合为 200（证明折叠生效）"
    );
    assert_eq!(
        metrics.by_turn_records.get("turn-wave54-t3-metrics").map(Vec::len),
        Some(2),
        "T3：正向对照——per-turn records 必须有 2 条（否则 first-wins 断言可能空转）"
    );
}

// ================================ T7 · 生产接线覆盖（L3-B ADVISORY-4 / B1 整改）

const T7_MODEL_FIRST: &str = "wave54-t7-auto-call-1";
const T7_MODEL_SECOND: &str = "wave54-t7-auto-call-2";
const T7_MODEL_INVALID: &str = "wave54-t7-auto-call-invalid";

const T7_FIRST_DURATION_MS: u64 = 1_200;
const T7_FIRST_TTFT_MS: u64 = 200;
const T7_FIRST_INPUT_TOKENS: u64 = 1_000;
const T7_FIRST_OUTPUT_TOKENS: u64 = 100;
const T7_SECOND_DURATION_MS: u64 = 1_800;
const T7_SECOND_INPUT_TOKENS: u64 = 2_000;
const T7_SECOND_OUTPUT_TOKENS: u64 = 200;
/// 无效调用档：有耗时、无用量 ⇒ 不得计入 `calls` / `Σout` / `Σgen`。
const T7_INVALID_DURATION_MS: u64 = 2_000;

/// T7（L3-B ADVISORY-4）：**生产接线覆盖** —— 根 `agent.turn` 的 C2 真值必须由
/// provider 侧守卫的既有调用**自行累加**得出，不得依赖调用方在每个终态出口逐点接线。
///
/// 装配方式（关键）：
/// - `begin_turn_trace` 开根 span；
/// - 两跳只调用 per-call 守卫（生产 provider 路径的真实接线）的
///   `record_usage(Some(in), Some(out))` + `record_call_timing(Some(dur), Some(ttft))`；
/// - **故意不调用** `TurnTraceGuard::record_turn_speed` / `record_turn_timing`
///   —— 若根 span 仍能给出速度/basis/calls/ttft，即证明累加口径落在接线内部（B1 目标）；
/// - 叠加一条无效调用（`record_usage(None, None)` + `record_call_timing(Some(2000), None)`）
///   确认 `calls` 仍为 2、速度不变（无效调用不得污染 Σ）。
///
/// 真值：① gen = 1200 − 200 = 1000ms；② 无 ttft ⇒ gen = 1800ms
/// ⇒ Σout = 300、Σgen = 2800ms ⇒ speed = 300 / 2.8 = 107.142857… t/s；turn ttft = 首跳 200ms。
#[test]
fn t7_root_span_accumulates_without_explicit_turn_calls() {
    let _guard = gate_lock();

    let expected_speed = (T7_FIRST_OUTPUT_TOKENS + T7_SECOND_OUTPUT_TOKENS) as f64
        / (((T7_FIRST_DURATION_MS - T7_FIRST_TTFT_MS) + T7_SECOND_DURATION_MS) as f64 / 1_000.0);
    // 若无效调用被误计入 Σ（Σgen = 1000 + 1800 + 2000），速度会掉到 62.5 t/s。
    let polluted_speed = (T7_FIRST_OUTPUT_TOKENS + T7_SECOND_OUTPUT_TOKENS) as f64
        / (((T7_FIRST_DURATION_MS - T7_FIRST_TTFT_MS) + T7_SECOND_DURATION_MS + T7_INVALID_DURATION_MS)
            as f64
            / 1_000.0);
    assert!(
        (expected_speed - 107.14285714285714).abs() <= 1e-6,
        "T7 夹具自检：两跳加权速度必须是 107.14285714285714 t/s，实际 {expected_speed}"
    );

    let spans = run_export(Duration::from_millis(2_500), || {
        let turn = otel::begin_turn_trace("sess-wave54-t7", "turn-wave54-t7", 7);

        // ① 首跳：usage 必须先于 call_timing（后者结算时需要本次 output）。
        let mut first = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T7_MODEL_FIRST),
            "chat",
        );
        first.record_usage(Some(T7_FIRST_INPUT_TOKENS), Some(T7_FIRST_OUTPUT_TOKENS));
        first.record_call_timing(Some(T7_FIRST_DURATION_MS), Some(T7_FIRST_TTFT_MS));
        first.record_usage_origin(Some("provider"));
        drop(first);

        // ② 第二跳：无首包（ttft 缺失）⇒ gen = dur = 1800ms。
        let mut second = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T7_MODEL_SECOND),
            "chat",
        );
        second.record_usage(Some(T7_SECOND_INPUT_TOKENS), Some(T7_SECOND_OUTPUT_TOKENS));
        second.record_call_timing(Some(T7_SECOND_DURATION_MS), None);
        second.record_usage_origin(Some("provider"));
        drop(second);

        // ③ 无效调用：usage 全空 + 有耗时无输出 ⇒ 不是「已完成调用」。
        let mut invalid = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T7_MODEL_INVALID),
            "chat",
        );
        invalid.record_usage(None, None);
        invalid.record_call_timing(Some(T7_INVALID_DURATION_MS), None);
        drop(invalid);

        // ★ 故意不调用 `turn.record_turn_speed(...)` / `turn.record_turn_timing(...)`：
        //   根 span 的 C2 真值必须在 Drop 时由累加口径自行结算。
        drop(turn);
    });

    // 正向对照：三条 `llm.chat` 子 span 确实导出了（否则下面的根 span 断言可能只是「没导出」）。
    let chats = find_spans_named(&spans, "llm.chat");
    assert_eq!(
        chats.len(),
        3,
        "T7 正向对照：本用例必须导出恰好 3 个 `llm.chat` span，实际 {}；导出 span 全量：{:?}",
        chats.len(),
        spans
            .iter()
            .map(|span| (span.name.as_str(), span_keys(span)))
            .collect::<Vec<_>>()
    );

    let root = find_span_with(&spans, "agent.turn", KEY_TURN_ID, "turn-wave54-t7");

    // ---------- C2 核心：速度 == Σout/(Σgen/1000)，无需任何显式 turn 级接线 ----------
    let wire_speed = wire_f64(root, KEY_TURN_SPEED).unwrap_or_else(|| {
        panic!(
            "T7（BLOCKING / B1 未收口）：根 `agent.turn` 必须在无显式 `record_turn_speed` 时自行累加输出 `{KEY_TURN_SPEED}`；属性全集：{:?}",
            span_keys(root)
        )
    });
    assert_close(
        wire_speed,
        expected_speed,
        &format!("T7（C2）：`{KEY_TURN_SPEED}` 必须等于两层 per-call 累加的 Σout/(Σgen/1000)"),
    );
    assert!(
        (wire_speed - polluted_speed).abs() > 1e-6,
        "T7：无效调用（dur=2000 无输出）不得进入 Σ，速度不得被稀释成 {polluted_speed}，实际 {wire_speed}"
    );
    assert!(
        wire_speed.is_finite() && wire_speed > 0.0,
        "T7：速度必须是有限正值，实际 {wire_speed}"
    );

    // ---------- C2：basis / calls / ttft / duration ----------
    assert_eq!(
        require_str(root, KEY_TURN_SPEED_BASIS, "T7（C2）"),
        "turn_weighted",
        "T7（C2）：累加口径必须自带 `turn_weighted` 标记"
    );
    assert_eq!(
        require_i64(root, KEY_TURN_CALLS, "T7（C2）"),
        2,
        "T7（C2）：`agent.turn.calls` 必须是两条**有效**已完成调用（无效调用不得计入）"
    );
    assert_eq!(
        require_i64(root, KEY_TURN_TTFT, "T7（C2）"),
        T7_FIRST_TTFT_MS as i64,
        "T7（C2）：`agent.turn.ttft_ms` 必须是首个带 ttft 的 per-call 值（首跳 200ms）"
    );
    // Lead 裁决（2026-10-27）：`agent.turn.duration_ms` 语义为「墙钟耗时」，但取下限
    // `max(elapsed, Σdur)` —— 生产环境取墙钟，下界为调用耗时之和，防止 0 值。
    // 本用例为合成导出（墙钟 < 1ms），故确定取下界 Σdur = 1200 + 1800 = 3000ms
    // （第 3 条无效调用 dur=2000 无 usage，不得进入 Σdur）。
    assert_eq!(
        require_i64(root, KEY_TURN_DURATION, "T7（C2）"),
        (T7_FIRST_DURATION_MS + T7_SECOND_DURATION_MS) as i64,
        "T7（C2）：`{KEY_TURN_DURATION}` 必须取 max(墙钟, Σdur) 的下界 3000ms（有效调用耗时之和，无效调用不计）"
    );

    // 正向对照：根 span 的聚合用量确实来自两条有效调用（100 + 200 = 300）。
    assert_eq!(
        require_i64(root, KEY_GEN_AI_OUTPUT, "T7 正向对照"),
        (T7_FIRST_OUTPUT_TOKENS + T7_SECOND_OUTPUT_TOKENS) as i64,
        "T7 正向对照：根 span 的 `gen_ai.usage.output_tokens` 必须聚合为 300（证明累加链路生效）"
    );
}
