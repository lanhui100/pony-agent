//! wave-53 冻结验收测试 · 只读 · 依据 .dev-team/contract-matrix-wave53.md
//!
//! 归属：**Test Agent 独占写域**（独立测试分权）。契约来源：
//! - `.dev-team/contract-matrix-wave53.md` §1 冻结契约 C1~C6
//! - `.dev-team/contract-matrix-wave53.md` §2 验收用例 T1~T6
//! - 驱动事实：`docs/audit/observability-audit-2026-10-27b.md`（真实会话 pa_01a17c85 / trace eed9ba47）
//!
//! 本文件一经冻结即**只读**：Executor 不得修改任何断言、不得删除用例、不得 `#[ignore]`。
//! 红相（stub `unimplemented!()` panic）是预期状态，不允许为了让测试变绿而放宽断言。
//!
//! 覆盖（零外网；全部走回环 OTLP sink 的线级（wire）解码，或纯函数值级断言）：
//! - **T1** 有价成本三分量：`record_cost_components(Some(0.001), Some(0.002), Some(0.003))` 后
//!   `llm.chat` 上 `langfuse.observation.cost_details` 必须是可 `serde_json::from_str` 的单行 JSON，
//!   `input/output/total` ≈ 0.001/0.002/0.003（±1e-9）；`gen_ai.cost.usd`≈0.003 仍须存在（Tempo 面不回归）；
//!   根 `agent.turn` 的 `gen_ai.cost.usd` 聚合 == 两次调用之和（0.006）。
//! - **T2** 未计价显式标记：仅 `record_usage` ⇒ `pony.cost.priced=false` 且**无** `cost_details`；
//!   记录成本 ⇒ `true`；usage 与成本都未记录 ⇒ 两键**均不存在**（禁止 0/false 占位）。
//! - **T3** 枚举收敛：`llm_span_attributes` 的 `gen_ai.system` 走 `gen_ai_system_enum` 映射
//!   （`openai-completions`/`openai-responses`→`openai`、`anthropic-messages`→`anthropic`、
//!   `gemini-2.5-pro`→`google`、`PonyLlm`→`generic`），`gen_ai.provider.name` 保留展示名。
//! - **T4** degraded 语义拆键：`completed` + 1 个 failed 工具 ⇒ `agent.turn.degraded=true` +
//!   `agent.turn.tool_failures=1`；全 success ⇒ 两键均不存在；`phase="error"` + 1 个 failed 工具 ⇒
//!   有 `tool_failures`、**无** `degraded`（错误语义仍由 outcome/status 表达）。
//! - **T5** 可读 trace 名：`turn_span_attributes(sequence=7)` ⇒ `langfuse.trace.name == "pony-agent#turn-7"`。
//! - **T6** tool call id：`call_id=Some("call_1")` ⇒ `gen_ai.tool.call.id=="call_1"`；`None` ⇒ 键不存在。
//!
//! 契约演进（2026-10-27 17:35 Lead 裁决 = 方案 a「附加式 API，历史冻结工件零改动」）：
//! C3/C6 的落地形态改为**附加入口** —— `start_llm_span_with_provider` /
//! `llm_span_attributes_with_provider` / `tool_span_attributes_with_call_id` /
//! `record_tool_call_span_with_call_id`；既有 3 参 `start_llm_span` 与
//! `LlmSpanInput` / `ToolSpanInput` 结构体**零变更**（否则 6 个历史冻结测试目标 E0061/E0063 编译红）。
//! 本文件只调用附加入口，**不触碰** `src/` 与任何历史冻结测试文件。
//!
//! 替代理由（硬约束「无法纯单元/线级验证时改纯函数或 wire 解码」）：
//! 1. 真实 Langfuse/Tempo 网络摄取不可用 ⇒ 全部改为**回环 OTLP sink** 的 protobuf wire 解码断言
//!    （`~90` 行最小解码器，不新增依赖、不进 `src/`），键名/类型/数值均在 span 自身属性级判定。
//! 2. 真实 turn 编排（`turn_flow.rs` / `runtime/turn_*.rs`）需要网络 provider ⇒ 改为
//!    `begin_turn_trace` + `record_tool_call_span` + `set_phase` 的最小 turn 装配，等价驱动 C4 的
//!    「活跃 turn 内失败计数 → 根 span 输出」路径。
//! 3. `gen_ai_system_enum` / `llm_span_attributes` / `tool_span_attributes` / `turn_span_attributes`
//!    均为无网络、无全局态的**纯函数** ⇒ 直接值级断言（不受门禁/init 影响，红相定位更精确）。
//!
//! 断言纪律：
//! - 属性键一律用**字面量**断言（避免「常量被改后自指自证」），另设常量字面量冻结断言；
//! - 禁止 `assert!(true)` 式同义反复、禁止 `#[ignore]`、禁止吞异常、禁止 skip；
//! - 每个「键不存在」断言都配**正向对照**（同 span 的既有键必须解出），避免「根本没导出/
//!   根本没解到 span」被误判成「键缺失」；
//! - 全局门禁 / 全局 trace provider 是进程级单例，凡触达它们的用例一律在 `gate_lock()` 串行下执行，
//!   每条用例结束 `otel::shutdown()` + 关闭门禁复位。

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::otel::{
    self, AttrValue, LlmSpanInput, ToolSpanInput, TurnSpanInput, ATTR_AGENT_TURN_DEGRADED,
    ATTR_AGENT_TURN_TOOL_FAILURES, ATTR_GEN_AI_PROVIDER_NAME, ATTR_GEN_AI_TOOL_CALL_ID,
    ATTR_LANGFUSE_OBSERVATION_COST_DETAILS, ATTR_PONY_COST_PRICED, DEFAULT_SAMPLING_RATIO,
};

// ---------------------------------------------------------------- 门禁串行锁
//
// 全局门禁是进程级 `AtomicBool`、全局 trace provider 是进程级单例，`cargo test` 默认多线程并发。
// 凡触达门禁 / 全局 provider 的用例必须持锁串行，并在结束处复位（与 wave-51 冻结文件同构）。

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
        service_name: "pony-agent-wave53-acceptance".to_string(),
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
        "wave-53：本地回环端点必须可装配（装配失败只降级不 panic）：{assembled:?}"
    );
    otel::set_reporting_enabled(true);

    body();

    let flushed = otel::force_flush(3_000);
    assert!(
        flushed.is_ok(),
        "wave-53：已装配时 force_flush 必须成功（否则下述缺键断言可能只是「没导出」）：{flushed:?}"
    );
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(
        !captured.is_empty(),
        "wave-53：必须至少导出一次 POST /v1/traces，否则线级断言无意义（捕获：{:?}）",
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

/// 整型属性：必须是 `int_value`（落成字符串会破坏 Langfuse/看板数值过滤）。
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

/// 布尔属性：必须是 `bool_value`。
fn wire_bool(span: &DecodedSpan, key: &str) -> Option<bool> {
    match wire_attr(span, key) {
        None => None,
        Some(DecodedValue::Bool(value)) => Some(*value),
        Some(other) => panic!(
            "span `{}` 的属性 `{key}` 必须以布尔（bool_value）上 wire，实际 {other:?}；属性全集：{:?}",
            span.name,
            span_keys(span)
        ),
    }
}

/// 断言键**不存在**（缺键 ≠ 0/false 占位）。
fn assert_key_absent(span: &DecodedSpan, key: &str) {
    assert!(
        wire_attr(span, key).is_none(),
        "span `{}` 不得输出 `{key}`（禁止 0/空串/false 占位），实际 {:?}；属性全集：{:?}",
        span.name,
        wire_attr(span, key),
        span_keys(span)
    );
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

// ---------------------------------------------------------------- 纯函数属性断言小工具

fn attr<'a>(attrs: &'a [(String, AttrValue)], key: &str) -> Option<&'a AttrValue> {
    attrs.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

fn pure_keys(attrs: &[(String, AttrValue)]) -> Vec<&str> {
    attrs.iter().map(|(name, _)| name.as_str()).collect()
}

fn pure_str<'a>(attrs: &'a [(String, AttrValue)], key: &str) -> Option<&'a str> {
    match attr(attrs, key) {
        None => None,
        Some(AttrValue::Str(value)) => Some(value.as_str()),
        Some(other) => panic!(
            "属性 `{key}` 必须以字符串（AttrValue::Str）输出，实际 {other:?}；属性全集：{:?}",
            pure_keys(attrs)
        ),
    }
}

fn pure_i64(attrs: &[(String, AttrValue)], key: &str) -> Option<i64> {
    match attr(attrs, key) {
        None => None,
        Some(AttrValue::I64(value)) => Some(*value),
        Some(other) => panic!(
            "属性 `{key}` 必须以整数（AttrValue::I64）输出，实际 {other:?}；属性全集：{:?}",
            pure_keys(attrs)
        ),
    }
}

/// 浮点近似断言（±1e-9；成本/金额不能用 `==` 比）。
fn assert_close(actual: f64, expected: f64, label: &str) {
    let delta = (actual - expected).abs();
    assert!(
        delta <= 1e-9,
        "{label}：期望 {expected}（±1e-9），实际 {actual}（偏差 {delta:e}）"
    );
}

// ==================================================== 常量字面量冻结（防自指自证）

/// 契约 C1~C6 新增的 6 个属性键必须与 `.dev-team/contract-matrix-wave53.md` §1 逐字一致。
///
/// 若只断言「常量 == 常量」，把键名改成任意字符串都能过；因此此处钉**字面量**。
#[test]
fn t0_attribute_key_literals_frozen() {
    assert_eq!(
        ATTR_LANGFUSE_OBSERVATION_COST_DETAILS, "langfuse.observation.cost_details",
        "C1：成本三分量键名必须逐字为 `langfuse.observation.cost_details`"
    );
    assert_eq!(
        ATTR_PONY_COST_PRICED, "pony.cost.priced",
        "C2：未计价标记键名必须逐字为 `pony.cost.priced`"
    );
    assert_eq!(
        ATTR_GEN_AI_PROVIDER_NAME, "gen_ai.provider.name",
        "C3：供应商展示名键名必须逐字为 `gen_ai.provider.name`"
    );
    assert_eq!(
        ATTR_AGENT_TURN_DEGRADED, "agent.turn.degraded",
        "C4：降级标记键名必须逐字为 `agent.turn.degraded`"
    );
    assert_eq!(
        ATTR_AGENT_TURN_TOOL_FAILURES, "agent.turn.tool_failures",
        "C4：工具失败计数键名必须逐字为 `agent.turn.tool_failures`"
    );
    assert_eq!(
        ATTR_GEN_AI_TOOL_CALL_ID, "gen_ai.tool.call.id",
        "C6：工具调用 id 键名必须逐字为 `gen_ai.tool.call.id`"
    );
}

// ============================================================ T1 · 有价成本三分量

const KEY_COST_DETAILS: &str = "langfuse.observation.cost_details";
const KEY_PRICED: &str = "pony.cost.priced";
const KEY_PROVIDER_NAME: &str = "gen_ai.provider.name";
const KEY_TURN_DEGRADED: &str = "agent.turn.degraded";
const KEY_TURN_TOOL_FAILURES: &str = "agent.turn.tool_failures";
const KEY_TOOL_CALL_ID: &str = "gen_ai.tool.call.id";
const KEY_GEN_AI_COST_USD: &str = "gen_ai.cost.usd";
const KEY_GEN_AI_SYSTEM: &str = "gen_ai.system";
const KEY_GEN_AI_OPERATION: &str = "gen_ai.operation.name";
const KEY_GEN_AI_REQUEST_MODEL: &str = "gen_ai.request.model";
const KEY_GEN_AI_USAGE_INPUT: &str = "gen_ai.usage.input_tokens";
const KEY_AGENT_TURN_ID: &str = "agent.turn.id";
const KEY_AGENT_TURN_PHASE: &str = "agent.turn.phase";
const KEY_TOOL_NAME: &str = "gen_ai.tool.name";
const KEY_TRACE_NAME: &str = "langfuse.trace.name";

const INPUT_COST: f64 = 0.001;
const OUTPUT_COST: f64 = 0.002;
const TOTAL_COST: f64 = 0.003;
const T1_MODEL: &str = "wave53-t1-cost-model";

/// T1：`cost_details` 必须是可解析 JSON，三分量数值与入参一致；`gen_ai.cost.usd` 不退场；
/// 根 `agent.turn` 的成本聚合 == 两次调用之和（不得漏算或重复计）。
#[test]
fn t1_cost_components_wire() {
    let _guard = gate_lock();

    let spans = run_export(Duration::from_millis(2_500), || {
        let _turn = otel::begin_turn_trace("sess-wave53-t1", "turn-wave53-t1", 1);
        for _ in 0..2 {
            let mut span = otel::start_llm_span_with_provider(
                "openai-completions",
                Some("PonyLlm"),
                Some(T1_MODEL),
                "chat",
            );
            span.record_usage(Some(100), Some(50));
            span.record_cost_components(Some(INPUT_COST), Some(OUTPUT_COST), Some(TOTAL_COST));
            drop(span);
        }
    });

    let chats = find_spans_named(&spans, "llm.chat");
    assert_eq!(
        chats.len(),
        2,
        "T1：本用例必须导出恰好 2 个 `llm.chat` span，实际 {}；导出 span 全量：{:?}",
        chats.len(),
        spans
            .iter()
            .map(|span| (span.name.as_str(), span_keys(span)))
            .collect::<Vec<_>>()
    );

    for chat in &chats {
        // 正向对照：先证明这是本用例造的 `llm.chat`（否则后面的取值断言可能命中了别的 span）。
        assert_eq!(
            wire_str(chat, KEY_GEN_AI_REQUEST_MODEL),
            Some(T1_MODEL),
            "T1：`llm.chat` 必须是本用例造出的调用（model={T1_MODEL}），属性全集：{:?}",
            span_keys(chat)
        );

        // ① cost_details 必须是**字符串**且可被 serde_json 解析（Langfuse 只认 JSON 字符串）。
        let raw = wire_str(chat, KEY_COST_DETAILS).unwrap_or_else(|| {
            panic!(
                "T1（BLOCKING）：`llm.chat` 必须输出 `{KEY_COST_DETAILS}`（Langfuse 成本语义轨）；属性全集：{:?}",
                span_keys(chat)
            )
        });
        let parsed: serde_json::Value = serde_json::from_str(raw).unwrap_or_else(|err| {
            panic!("T1：`{KEY_COST_DETAILS}` 必须是合法 JSON 字符串（实际 `{raw}`）：{err}")
        });
        let object = parsed
            .as_object()
            .unwrap_or_else(|| panic!("T1：`{KEY_COST_DETAILS}` 必须是 JSON 对象，实际 `{raw}`"));

        // ② 键只允许 input / output / total（夹带其它键会污染 Langfuse 成本归因）。
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["input", "output", "total"],
            "T1：`{KEY_COST_DETAILS}` 只允许出现 input/output/total 三键，实际 `{raw}`"
        );

        // ③ 三分量数值（±1e-9）：input≈0.001 / output≈0.002 / total≈0.003。
        for (part, expected) in [
            ("input", INPUT_COST),
            ("output", OUTPUT_COST),
            ("total", TOTAL_COST),
        ] {
            let value = object
                .get(part)
                .and_then(serde_json::Value::as_f64)
                .unwrap_or_else(|| {
                    panic!("T1：`{KEY_COST_DETAILS}.{part}` 必须是数值，实际 `{raw}`")
                });
            assert_close(
                value,
                expected,
                &format!("T1：`{KEY_COST_DETAILS}.{part}` 必须等于入参 {expected}（JSON `{raw}`）"),
            );
        }

        // ④ 既有 `gen_ai.cost.usd`（Tempo 面）不得因新增三分量而消失。
        let cost = wire_f64(chat, KEY_GEN_AI_COST_USD).unwrap_or_else(|| {
            panic!(
                "T1：`llm.chat` 必须仍输出 `{KEY_GEN_AI_COST_USD}`（Tempo 面零回归）；属性全集：{:?}",
                span_keys(chat)
            )
        });
        assert_close(
            cost,
            TOTAL_COST,
            "T1：`gen_ai.cost.usd` 必须等于 total 分量 0.003",
        );
    }

    // ⑤ 根 `agent.turn` 的成本聚合 == 两次调用之和（0.003 + 0.003 = 0.006），
    //    既不得漏算（record_cost_components 未计入聚合），也不得重复计（与 record_cost 双计）。
    let root = find_span_with(&spans, "agent.turn", KEY_AGENT_TURN_ID, "turn-wave53-t1");
    let root_cost = wire_f64(root, KEY_GEN_AI_COST_USD).unwrap_or_else(|| {
        panic!(
            "T1：根 `agent.turn` 必须输出聚合 `{KEY_GEN_AI_COST_USD}`；属性全集：{:?}",
            span_keys(root)
        )
    });
    assert_close(
        root_cost,
        TOTAL_COST * 2.0,
        "T1：根 `agent.turn` 的 `gen_ai.cost.usd` 必须为两次调用之和（0.006，禁漏算/禁双计）",
    );
}

// ============================================================ T2 · 未计价显式标记

const T2_MODEL_USAGE_ONLY: &str = "wave53-t2-usage-only";
const T2_MODEL_PRICED: &str = "wave53-t2-priced";
const T2_MODEL_BARE: &str = "wave53-t2-bare";

/// T2：`pony.cost.priced` 三态 —— 有 usage 无成本 ⇒ `false`；有成本 ⇒ `true`；
/// usage 与成本皆无 ⇒ 该键与 `cost_details` **都不存在**（禁止 false/空 JSON 占位）。
#[test]
fn t2_priced_marker_wire() {
    let _guard = gate_lock();

    let spans = run_export(Duration::from_millis(2_500), || {
        let _turn = otel::begin_turn_trace("sess-wave53-t2", "turn-wave53-t2", 2);

        // ① 只记 usage（未计价）。
        let mut usage_only = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T2_MODEL_USAGE_ONLY),
            "chat",
        );
        usage_only.record_usage(Some(10), Some(5));
        drop(usage_only);

        // ② 记了成本（已计价）。
        let mut priced = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T2_MODEL_PRICED),
            "chat",
        );
        priced.record_usage(Some(10), Some(5));
        priced.record_cost(Some(0.004));
        drop(priced);

        // ③ usage 与成本都没有（无采集结果）。
        let bare = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(T2_MODEL_BARE),
            "chat",
        );
        drop(bare);
    });

    // ① 未计价：priced=false，且不得输出 cost_details（否则 Langfuse 会把 0 成本当真值）。
    let usage_only = find_span_with(&spans, "llm.chat", KEY_GEN_AI_REQUEST_MODEL, T2_MODEL_USAGE_ONLY);
    assert_eq!(
        wire_i64(usage_only, KEY_GEN_AI_USAGE_INPUT),
        Some(10),
        "T2①：正向对照——未计价 span 必须来自真实 `record_usage` 出口（input=10）；属性全集：{:?}",
        span_keys(usage_only)
    );
    assert_eq!(
        wire_bool(usage_only, KEY_PRICED),
        Some(false),
        "T2①（BLOCKING）：有 usage 但未记录成本时必须显式标记 `pony.cost.priced=false`；属性全集：{:?}",
        span_keys(usage_only)
    );
    assert_key_absent(usage_only, KEY_COST_DETAILS);

    // ② 已计价：priced=true，且 `gen_ai.cost.usd` 仍在（正向对照）。
    let priced = find_span_with(&spans, "llm.chat", KEY_GEN_AI_REQUEST_MODEL, T2_MODEL_PRICED);
    assert_eq!(
        wire_bool(priced, KEY_PRICED),
        Some(true),
        "T2②：记录过成本时必须标记 `pony.cost.priced=true`；属性全集：{:?}",
        span_keys(priced)
    );
    let priced_cost = wire_f64(priced, KEY_GEN_AI_COST_USD).unwrap_or_else(|| {
        panic!(
            "T2②：正向对照——已计价 span 必须带 `{KEY_GEN_AI_COST_USD}`；属性全集：{:?}",
            span_keys(priced)
        )
    });
    assert_close(priced_cost, 0.004, "T2②：`gen_ai.cost.usd` 必须等于入参 0.004");

    // ③ 两无：两个键都不得出现（「未知」与「未计价」是两种语义，不能混为一谈）。
    let bare = find_span_with(&spans, "llm.chat", KEY_GEN_AI_REQUEST_MODEL, T2_MODEL_BARE);
    assert_eq!(
        wire_str(bare, KEY_GEN_AI_OPERATION),
        Some("chat"),
        "T2③：正向对照——裸 span 自身属性必须可解码（operation=chat）；属性全集：{:?}",
        span_keys(bare)
    );
    assert_key_absent(bare, KEY_PRICED);
    assert_key_absent(bare, KEY_COST_DETAILS);
}

// ============================================================ T3 · gen_ai.system 枚举收敛

/// T3：`gen_ai_system_enum` 是 `gen_ai.system` 的唯一映射源（幂等），且展示名下沉 `gen_ai.provider.name`。
#[test]
fn t3_gen_ai_system_enum_convergence() {
    // ① 直接钉纯函数映射表（C3 逐条）。
    for (label, expected) in [
        ("openai-completions", "openai"),
        ("openai-responses", "openai"),
        ("anthropic-messages", "anthropic"),
        ("gemini-2.5-pro", "google"),
        ("google-vertex", "google"),
        ("PonyLlm", "generic"),
    ] {
        assert_eq!(
            otel::gen_ai_system_enum(label),
            expected,
            "T3：`gen_ai_system_enum({label})` 必须收敛为 `{expected}`"
        );
    }

    // ② 幂等：入参本身已是枚举值时必须原样返回（否则调用点二次映射会漂移）。
    for canonical in ["openai", "anthropic", "google", "generic"] {
        assert_eq!(
            otel::gen_ai_system_enum(canonical),
            canonical,
            "T3：`{canonical}` 已是枚举值，映射必须幂等"
        );
    }

    // ③ `llm_span_attributes_with_provider`：协议标签进、枚举值出；展示名走 `gen_ai.provider.name`。
    let llm_input = LlmSpanInput {
        provider_kind: "openai-completions".to_string(),
        model: Some("gpt-4o".to_string()),
        input_tokens: Some(100),
        output_tokens: Some(50),
        operation: "chat".to_string(),
    };
    let attributes = otel::llm_span_attributes_with_provider(&llm_input, Some("PonyLlm"));
    assert_eq!(
        pure_str(&attributes, KEY_GEN_AI_SYSTEM),
        Some("openai"),
        "T3（BLOCKING）：`gen_ai.system` 必须是枚举值 `openai`（协议标签不得直接上 wire）；属性全集：{:?}",
        pure_keys(&attributes)
    );
    assert_eq!(
        pure_str(&attributes, KEY_PROVIDER_NAME),
        Some("PonyLlm"),
        "T3：`gen_ai.provider.name` 必须保留展示名 `PonyLlm`（不得被枚举化）;属性全集：{:?}",
        pure_keys(&attributes)
    );
    // 正向对照：既有属性一个都不能少（改映射不得破坏其它键）。
    assert_eq!(pure_str(&attributes, KEY_GEN_AI_REQUEST_MODEL), Some("gpt-4o"));
    assert_eq!(pure_i64(&attributes, KEY_GEN_AI_USAGE_INPUT), Some(100));
    assert_eq!(pure_str(&attributes, KEY_GEN_AI_OPERATION), Some("chat"));

    // ④ 无展示名 ⇒ 不得输出 `gen_ai.provider.name`（禁止空串占位）。
    let without_name = otel::llm_span_attributes_with_provider(
        &LlmSpanInput {
            provider_kind: "anthropic-messages".to_string(),
            model: None,
            input_tokens: None,
            output_tokens: None,
            operation: "chat".to_string(),
        },
        None,
    );
    assert_eq!(
        pure_str(&without_name, KEY_GEN_AI_SYSTEM),
        Some("anthropic"),
        "T3：`anthropic-messages` 必须收敛为 `anthropic`"
    );
    assert!(
        attr(&without_name, KEY_PROVIDER_NAME).is_none(),
        "T3：展示名缺失时不得输出 `gen_ai.provider.name`（禁止空串），实际属性全集：{:?}",
        pure_keys(&without_name)
    );

    // ⑤ 旧入口（`llm_span_attributes`，无展示名）也必须走同一份枚举映射 ——
    //    否则 turn_flow 漏改一处调用点就会把协议标签原样送上 wire（不可证伪的隐性回退）。
    let legacy = otel::llm_span_attributes(&llm_input);
    assert_eq!(
        pure_str(&legacy, KEY_GEN_AI_SYSTEM),
        Some("openai"),
        "T3：既有 `llm_span_attributes` 也必须收敛为 `openai`（旧入口等价于传 `None` 委托）；属性全集：{:?}",
        pure_keys(&legacy)
    );
    assert!(
        attr(&legacy, KEY_PROVIDER_NAME).is_none(),
        "T3：既有 `llm_span_attributes` 无展示名 ⇒ 不得输出 `gen_ai.provider.name`，实际属性全集：{:?}",
        pure_keys(&legacy)
    );
}

// ============================================================ T4 · degraded 语义拆键

fn tool_input(tool_name: &str, status: &str, error: Option<&str>) -> ToolSpanInput {
    ToolSpanInput {
        tool_name: tool_name.to_string(),
        status: status.to_string(),
        duration_ms: Some(120),
        error: error.map(|value| value.to_string()),
    }
}

/// T4：`completed` + 1 个 failed 工具 ⇒ `degraded=true` + `tool_failures=1`；
/// 全 success ⇒ 两键均不存在；`phase="error"` ⇒ 有 `tool_failures`、**无** `degraded`。
#[test]
fn t4_degraded_semantics_wire() {
    let _guard = gate_lock();

    let spans = run_export(Duration::from_millis(3_000), || {
        // ① completed + 1 failed + 1 success ⇒ 部分失败（degraded）。
        let mut degraded = otel::begin_turn_trace("sess-wave53-t4", "turn-t4-degraded", 0);
        // C6 附加式接线：`_with_call_id` 必须把模型侧 call id 一路带到 `tool.call` span。
        otel::record_tool_call_span_with_call_id(
            tool_input("bash", "failed", Some("exit code 1")),
            Some("call-wire-degraded".to_string()),
        );
        otel::record_tool_call_span(&tool_input("read_file", "success", None));
        degraded.set_phase("completed");
        drop(degraded);

        // ② 全 success 的 completed turn ⇒ 两个键都不得出现。
        let mut healthy = otel::begin_turn_trace("sess-wave53-t4", "turn-t4-healthy", 1);
        otel::record_tool_call_span(&tool_input("bash", "success", None));
        otel::record_tool_call_span(&tool_input("read_file", "success", None));
        healthy.set_phase("completed");
        drop(healthy);

        // ③ phase="error" + 1 failed ⇒ 只出计数，不出 degraded（错误语义由 outcome/status 表达）。
        let mut errored = otel::begin_turn_trace("sess-wave53-t4", "turn-t4-error", 2);
        otel::record_tool_call_span(&tool_input("bash", "failed", Some("exit code 2")));
        otel::record_tool_call_span(&tool_input("read_file", "success", None));
        errored.set_phase("error");
        drop(errored);
    });

    // 复核前置：三个 turn 的工具子 span 必须都真实上 wire（否则根聚合断言可能「无输入」）。
    let tool_spans = find_spans_named(&spans, "tool.call");
    let failed_tools = tool_spans
        .iter()
        .filter(|span| wire_str(span, "agent.tool.status") == Some("failed"))
        .count();
    let success_tools = tool_spans
        .iter()
        .filter(|span| wire_str(span, "agent.tool.status") == Some("success"))
        .count();
    assert_eq!(
        (failed_tools, success_tools),
        (2, 4),
        "T4：本用例必须导出 2 个 failed + 4 个 success 的 `tool.call` span，实际 {failed_tools}/{success_tools}；\
         导出 span 全量：{:?}",
        spans
            .iter()
            .map(|span| (span.name.as_str(), span_keys(span)))
            .collect::<Vec<_>>()
    );

    // 补充（C6 wire 侧）：`record_tool_call_span_with_call_id` 必须把 call id 真正送到线级。
    let wired = find_span_with(&spans, "tool.call", KEY_TOOL_CALL_ID, "call-wire-degraded");
    assert_eq!(
        wire_str(wired, "agent.tool.status"),
        Some("failed"),
        "T4/C6：带 `gen_ai.tool.call.id` 的 span 必须是本用例的 failed bash 调用；属性全集：{:?}",
        span_keys(wired)
    );

    // ① completed + 1 失败 ⇒ degraded=true 且 tool_failures=1。
    let root_degraded = find_span_with(&spans, "agent.turn", KEY_AGENT_TURN_ID, "turn-t4-degraded");
    assert_eq!(
        wire_str(root_degraded, KEY_AGENT_TURN_PHASE),
        Some("completed"),
        "T4①：正向对照——根 span 相位必须是 completed；属性全集：{:?}",
        span_keys(root_degraded)
    );
    assert_eq!(
        wire_i64(root_degraded, KEY_TURN_TOOL_FAILURES),
        Some(1),
        "T4①（BLOCKING）：completed 且 1 个工具失败时根 `agent.turn` 必须输出 `tool_failures=1`；\
         属性全集：{:?}",
        span_keys(root_degraded)
    );
    assert_eq!(
        wire_bool(root_degraded, KEY_TURN_DEGRADED),
        Some(true),
        "T4①（BLOCKING）：completed 且存在工具失败时必须标记 `agent.turn.degraded=true`；属性全集：{:?}",
        span_keys(root_degraded)
    );

    // ② 全 success ⇒ 两键都不存在（全健康轮次不得出现降级标记/0 计数）。
    let root_healthy = find_span_with(&spans, "agent.turn", KEY_AGENT_TURN_ID, "turn-t4-healthy");
    assert_eq!(
        wire_str(root_healthy, KEY_AGENT_TURN_PHASE),
        Some("completed"),
        "T4②：正向对照——健康轮次相位必须是 completed；属性全集：{:?}",
        span_keys(root_healthy)
    );
    assert_key_absent(root_healthy, KEY_TURN_TOOL_FAILURES);
    assert_key_absent(root_healthy, KEY_TURN_DEGRADED);

    // ③ phase="error" ⇒ 有计数、无 degraded（不得把「已失败」再降级成「部分失败」）。
    let root_error = find_span_with(&spans, "agent.turn", KEY_AGENT_TURN_ID, "turn-t4-error");
    assert_eq!(
        wire_str(root_error, KEY_AGENT_TURN_PHASE),
        Some("error"),
        "T4③：正向对照——错误轮次相位必须是 error；属性全集：{:?}",
        span_keys(root_error)
    );
    assert_eq!(
        wire_i64(root_error, KEY_TURN_TOOL_FAILURES),
        Some(1),
        "T4③：错误轮次的工具失败计数必须仍然输出（1）；属性全集：{:?}",
        span_keys(root_error)
    );
    assert_key_absent(root_error, KEY_TURN_DEGRADED);
}

// ============================================================ T5 · 可读 trace 名

/// T5：`langfuse.trace.name` 必须是 `pony-agent#turn-<sequence>`（十进制、无零填充）；
/// 同 span 上的 `agent.turn.id` / `agent.turn.sequence` 语义零变更。
#[test]
fn t5_trace_name_readable() {
    let attributes = otel::turn_span_attributes(&TurnSpanInput {
        session_id: "sess-wave53-t5".to_string(),
        user_id: None,
        turn_id: "turn-abc-raw-id".to_string(),
        sequence: 7,
        phase: "completed".to_string(),
    });

    assert_eq!(
        pure_str(&attributes, KEY_TRACE_NAME),
        Some("pony-agent#turn-7"),
        "T5（BLOCKING）：`langfuse.trace.name` 必须为 `pony-agent#turn-7`；属性全集：{:?}",
        pure_keys(&attributes)
    );
    // 负向对照：不得再回落到 turn_id（旧行为会让 Langfuse 列表出现不可读的内部 id）。
    assert_ne!(
        pure_str(&attributes, KEY_TRACE_NAME),
        Some("turn-abc-raw-id"),
        "T5：trace 名不得直接使用 turn_id"
    );
    // 既有键语义零变更。
    assert_eq!(
        pure_i64(&attributes, "agent.turn.sequence"),
        Some(7),
        "T5：`agent.turn.sequence` 必须保持入参序号 7"
    );
    assert_eq!(
        pure_str(&attributes, KEY_AGENT_TURN_ID),
        Some("turn-abc-raw-id"),
        "T5：`agent.turn.id` 必须保持原始 turn id"
    );
    assert_eq!(
        pure_str(&attributes, "langfuse.session.id"),
        Some("sess-wave53-t5"),
        "T5：`langfuse.session.id` 语义不变"
    );

    // 无零填充：sequence=10 必须写成 `#turn-10`（不是 `#turn-010`）。
    let padded = otel::turn_span_attributes(&TurnSpanInput {
        session_id: "sess-wave53-t5".to_string(),
        user_id: None,
        turn_id: "turn-seq-10".to_string(),
        sequence: 10,
        phase: "completed".to_string(),
    });
    assert_eq!(
        pure_str(&padded, KEY_TRACE_NAME),
        Some("pony-agent#turn-10"),
        "T5：序号必须十进制无零填充（`pony-agent#turn-10`），实际 {:?}",
        pure_str(&padded, KEY_TRACE_NAME)
    );
}

// ============================================================ T6 · tool call id

/// T6：`tool_span_attributes_with_call_id` 的 `call_id` 非空 ⇒ 输出 `gen_ai.tool.call.id`
/// （与模型侧 id 逐字相等）；`None` ⇒ 键不存在（禁止空串占位）。旧入口 `tool_span_attributes`
/// 零变更（等价于传 `None`），历史冻结测试不受影响。
#[test]
fn t6_tool_call_id() {
    let input = ToolSpanInput {
        tool_name: "bash".to_string(),
        status: "success".to_string(),
        duration_ms: Some(42),
        error: None,
    };

    let with_id = otel::tool_span_attributes_with_call_id(&input, Some("call_1"));
    assert_eq!(
        pure_str(&with_id, KEY_TOOL_CALL_ID),
        Some("call_1"),
        "T6（BLOCKING）：`call_id=Some(\"call_1\")` ⇒ `gen_ai.tool.call.id` 必须为 `call_1`；\
         属性全集：{:?}",
        pure_keys(&with_id)
    );
    // 正向对照：既有工具键不得因新增键而丢失。
    assert_eq!(pure_str(&with_id, KEY_TOOL_NAME), Some("bash"));
    assert_eq!(pure_str(&with_id, "agent.tool.status"), Some("success"));
    assert_eq!(pure_i64(&with_id, "agent.tool.duration_ms"), Some(42));

    let without_id = otel::tool_span_attributes_with_call_id(&input, None);
    assert!(
        attr(&without_id, KEY_TOOL_CALL_ID).is_none(),
        "T6：`call_id=None` ⇒ 不得输出 `gen_ai.tool.call.id`（禁止空串占位），实际属性全集：{:?}",
        pure_keys(&without_id)
    );
    // 正向对照：无 call_id 时仍必须输出工具名/状态（证明属性映射没整体失效）。
    assert_eq!(pure_str(&without_id, KEY_TOOL_NAME), Some("bash"));
    assert_eq!(pure_str(&without_id, "agent.tool.status"), Some("success"));

    // 旧入口零变更：`tool_span_attributes` 恒等价于 `None`（历史冻结工件的行为契约）。
    let legacy = otel::tool_span_attributes(&input);
    assert!(
        attr(&legacy, KEY_TOOL_CALL_ID).is_none(),
        "T6：既有 `tool_span_attributes` 不得输出 `gen_ai.tool.call.id`（等价于 `None`），实际：{:?}",
        pure_keys(&legacy)
    );
    assert_eq!(pure_str(&legacy, KEY_TOOL_NAME), Some("bash"));
}
