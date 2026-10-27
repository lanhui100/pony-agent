//! Wave51 · 可观测性上报修复验收测试（红相，冻结）。
//!
//! 归属：**Test Agent 独占写域**（独立测试分权）。契约来源：
//! - `.dev-team/contract-matrix-wave51.md` §1（PA51-C1 版本单一来源）/ §2（PA51-C2 治理会话归属）
//!   / §3（PA51-C3 思考 token 与缓存命中 token 出口）/ §7.2（OT-8e 授权）
//!
//! 覆盖（零外网；全部走回环 OTLP sink 或纯函数值级断言）：
//! 1. C1：`otel::init_with_service_version` 的 `service.version` 三级优先级（显式入参 > env >
//!    `CARGO_PKG_VERSION`）+ 空白回落 + 对抗（换行控制字符 / 65 字节超长）+ `PonySentryConfig::with_release`
//!    三态与脏值拒绝。
//! 2. C2：活跃 turn 内 `governance.decide` 必须带 `langfuse.session.id`；无活跃 turn 不得输出该键。
//! 3. C3：`llm_usage_detail_attributes` 纯函数值级契约（有值才出 / `Some(0)` 出 0 / `u64::MAX` 饱和）
//!    + wire 级键名出口 + 根 `agent.turn` 聚合口径 + 全 None 不出键。
//! 4. 接线静态断言（T6）：`turn_flow.rs` 的 `record_usage_details(` 计数 == 4；宿主装配入口与 `with_release`。
//! 5. L2-AT 对抗：并发双初始化 / 多次 export 不得 panic（`catch_unwind` 允许 `Err`，不允许 panic）。
//!
//! 断言纪律：属性键一律用**字面量**断言（避免「常量被改后自指自证」），另设常量字面量冻结断言；
//! 禁止 `#[ignore]`、禁止无断言、禁止吞异常；全局门禁/全局 provider 一律在 `gate_lock()` 串行下触达，
//! 每条用例结束 `otel::shutdown()` 复位。

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Barrier, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::otel::{
    self, AttrValue, ATTR_GEN_AI_USAGE_CACHED_INPUT_TOKENS, ATTR_GEN_AI_USAGE_REASONING_TOKENS,
    DEFAULT_SAMPLING_RATIO,
};
use pony_agent_core::agent::ponysentry::PonySentryConfig;

// ---------------------------------------------------------------- 门禁串行锁
//
// 全局门禁是进程级 `AtomicBool`、全局 trace provider 是进程级单例；cargo test 默认多线程并发跑
// 用例。凡触达门禁 / 全局 provider / 进程 env 的用例必须持锁串行，并在结束处复位。

static GATE_LOCK: Mutex<()> = Mutex::new(());

fn gate_lock() -> MutexGuard<'static, ()> {
    GATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// OTel `service.version` 的 env 覆盖入口（契约 PA51-C1：显式入参 > 本 env > crate 版本）。
const ENV_SERVICE_VERSION: &str = "PONY_OTEL_SERVICE_VERSION";

/// env 守卫：`set_var` + 用例结束 `remove_var`（防跨用例污染；用例已持 `gate_lock` 串行）。
struct EnvVarGuard {
    key: &'static str,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        std::env::set_var(key, value);
        Self { key }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        std::env::remove_var(self.key);
    }
}

// -------------------------------------------------- 本地 OTLP/HTTP sink（回环，零外网）
//
// 与 `tests/otel_trace_migration_test.rs` 的既有验收 harness 同构（非阻塞 accept + 固定轮询 +
// 硬截止，禁止无界等待）：捕获 method/path/body 后回 `200 + 空 ExportTraceServiceResponse`。
// 硬截止保证负向断言（键缺失 / 零 egress）不会悬空等待。

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

/// 既有 4 字段字面量（契约：本波**不**给 `OtelConfig` 加 `service_version` 字段）。
fn local_config(port: u16) -> otel::OtelConfig {
    otel::OtelConfig {
        endpoint: format!("http://127.0.0.1:{port}"),
        service_name: "pony-agent-wave51-acceptance".to_string(),
        sampling_ratio: DEFAULT_SAMPLING_RATIO,
        headers: Vec::new(),
    }
}

// ---------------------------------------------------------------- wire 断言小工具

/// 捕获请求摘要（失败信息里给出 method/path，避免「静默零导出」误判为「键缺失」）。
fn describe(captured: &[CapturedRequest]) -> Vec<(String, String)> {
    captured
        .iter()
        .map(|request| (request.method.clone(), request.path.clone()))
        .collect()
}

/// 合并全部导出请求的 body 文本（protobuf 里属性名/字符串值均为明文，可做包含性断言）。
fn wire_body(captured: &[CapturedRequest]) -> String {
    captured
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body).to_string())
        .collect::<Vec<_>>()
        .join("\n")
}

fn attr<'a>(attrs: &'a [(String, AttrValue)], key: &str) -> Option<&'a AttrValue> {
    attrs.iter().find(|(name, _)| name == key).map(|(_, value)| value)
}

/// 断言某键存在且为整数（落成字符串会破坏 Langfuse 数值过滤；落成负数说明 `as` 直转溢出）。
fn require_i64(attrs: &[(String, AttrValue)], key: &str) -> i64 {
    match attr(attrs, key) {
        Some(AttrValue::I64(value)) => *value,
        other => panic!(
            "属性 `{key}` 必须以整数（AttrValue::I64）输出，实际 {:?}；当前属性全集：{:?}",
            other,
            attrs.iter().map(|(name, _)| name).collect::<Vec<_>>()
        ),
    }
}

// ------------------------------------------------- 极简 protobuf wire 解码器（仅测试内）
//
// Lead 裁决 v4 授权：根 span 聚合数值（Σ / 0）不得只做「键名存在性」断言 —— wire 上整型属性是
// varint，必须反解出 int_value 才能钉住数值。故本文件内自写 ~90 行最小解码器（不新增依赖、
// 不进 src/）。只覆盖本波所需子集（字段号已对照 opentelemetry-proto `trace/v1/trace.proto` +
// `common/v1/common.proto` 核验）：
//   ExportTraceServiceRequest.resource_spans = 1
//   ResourceSpans.scope_spans = 2 / ScopeSpans.spans = 2
//   Span.name = 5 / Span.attributes = 9
//   KeyValue.key = 1 / KeyValue.value = 2
//   AnyValue.string_value = 1 / AnyValue.int_value = 3（varint）

enum WireValue {
    Varint(u64),
    Fixed64,
    Fixed32,
    Bytes(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq)]
enum DecodedValue {
    Int(i64),
    Str(String),
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
                cursor += 8;
                fields.push((number, WireValue::Fixed64));
            }
            2 => {
                let length = read_varint(bytes, &mut cursor) as usize;
                let end = cursor.saturating_add(length).min(bytes.len());
                fields.push((number, WireValue::Bytes(bytes[cursor..end].to_vec())));
                cursor = end;
            }
            5 => {
                cursor += 4;
                fields.push((number, WireValue::Fixed32));
            }
            // 本波不会出现 group（3/4），未知 wire type 直接停止解析，避免越界死循环。
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
            (3, WireValue::Varint(raw)) => return DecodedValue::Int(raw as i64),
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

/// 解出全部导出请求里的 span（可能跨多个 batch）。
fn decoded_spans(captured: &[CapturedRequest]) -> Vec<DecodedSpan> {
    let mut spans: Vec<DecodedSpan> = Vec::new();
    for request in captured {
        spans.extend(decode_spans(&request.body));
    }
    spans
}

fn find_span<'a>(spans: &'a [DecodedSpan], name: &str) -> &'a DecodedSpan {
    spans.iter().find(|span| span.name == name).unwrap_or_else(|| {
        panic!(
            "必须导出 span `{name}`，实际导出：{:?}",
            spans.iter().map(|span| span.name.as_str()).collect::<Vec<_>>()
        )
    })
}

/// 取整型属性值：键缺失 ⇒ `None`；键存在但类型不是整型 ⇒ panic（类型退化不得静默通过）。
fn span_int_attribute(span: &DecodedSpan, key: &str) -> Option<i64> {
    match span.attributes.iter().find(|(name, _)| name == key) {
        None => None,
        Some((_, DecodedValue::Int(value))) => Some(*value),
        Some((_, other)) => panic!(
            "span `{}` 的属性 `{key}` 必须以整型（int_value）上 wire，实际 {other:?}",
            span.name
        ),
    }
}

/// 取字符串属性值：键缺失 ⇒ `None`；键存在但类型不是字符串 ⇒ panic。
fn span_str_attribute<'a>(span: &'a DecodedSpan, key: &str) -> Option<&'a str> {
    match span.attributes.iter().find(|(name, _)| name == key) {
        None => None,
        Some((_, DecodedValue::Str(value))) => Some(value.as_str()),
        Some((_, other)) => panic!(
            "span `{}` 的属性 `{key}` 必须以字符串上 wire，实际 {other:?}",
            span.name
        ),
    }
}

/// C1 装配 + 单 turn 导出：`init_with_service_version(config, explicit)` → 造 span → `force_flush`
/// → `shutdown` → 返回 wire body 文本。调用方必须持 `gate_lock`。
fn export_wire_body_with_explicit_version(explicit: Option<&str>) -> String {
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(1_500));
    let assembled = otel::init_with_service_version(local_config(port), explicit);
    assert!(
        assembled.is_ok(),
        "PA51-C1：本地回环端点必须可装配（装配失败只降级不 panic）：{assembled:?}"
    );
    otel::set_reporting_enabled(true);

    {
        let _turn = otel::begin_turn_trace("sess-ver-wave51", "turn-ver-wave51", 0);
        let mut span = otel::start_llm_span("mock-provider", Some("gpt-4o"), "chat");
        span.record_usage(Some(100), Some(50));
    }

    let flushed = otel::force_flush(3_000);
    assert!(
        flushed.is_ok(),
        "PA51-C1：已装配时 force_flush 必须成功：{flushed:?}"
    );

    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(
        !captured.is_empty(),
        "PA51-C1：必须至少导出一次 POST /v1/traces，否则 resource 断言无意义（捕获：{:?}）",
        describe(&captured)
    );
    wire_body(&captured)
}

// ============================================================ PA51-C1 · 版本单一来源

/// PA51-C1-T1：显式入参非空 ⇒ resource `service.version` 恒等于该值（且压过 env / crate 版本）。
#[test]
fn c1_explicit_version_wins() {
    let _guard = gate_lock();

    let body = export_wire_body_with_explicit_version(Some("9.9.9"));

    assert!(
        body.contains("service.version"),
        "PA51-C1-T1：wire body 必须包含 resource 键 `service.version`"
    );
    assert!(
        body.contains("9.9.9"),
        "PA51-C1-T1：显式入参必须成为 `service.version` 取值（body 中找不到 `9.9.9`）"
    );
    assert_ne!(
        "9.9.9",
        env!("CARGO_PKG_VERSION"),
        "PA51-C1-T1：显式值与 crate 版本必须不同，否则无法证明优先级（否则断言自我满足）"
    );
}

/// PA51-C1-T2：显式入参纯空白 ⇒ 视为未设置，回落 env `PONY_OTEL_SERVICE_VERSION`。
#[test]
fn c1_blank_version_falls_back_to_env() {
    let _guard = gate_lock();
    let _env = EnvVarGuard::set(ENV_SERVICE_VERSION, "7.7.7");

    let body = export_wire_body_with_explicit_version(Some("   "));

    assert!(
        body.contains("service.version"),
        "PA51-C1-T2：wire body 必须包含 resource 键 `service.version`"
    );
    assert!(
        body.contains("7.7.7"),
        "PA51-C1-T2：空白显式入参必须回落到 env 值 `7.7.7`（不得写成空串）"
    );
    assert!(
        !body.contains("9.9.9"),
        "PA51-C1-T2：回落链不得混入其它版本值（body 中出现了 `9.9.9`）"
    );
}

/// PA51-C1-T4（对抗）：含控制字符（换行）与 65 字节超长值一律视为未设置，下落 crate 版本，
/// 不得把脏值写进 resource。
#[test]
fn c1_invalid_version_rejected() {
    let _guard = gate_lock();
    // 本用例验证「兜底一级 = crate 版本」，因此显式清掉 env 这一级。
    std::env::remove_var(ENV_SERVICE_VERSION);

    // ① 换行/控制字符注入：可污染 resource 与 Langfuse release 维度。
    let injected = "1.2.3\n9.9.9";
    let body_injected = export_wire_body_with_explicit_version(Some(injected));
    assert!(
        !body_injected.contains(injected),
        "PA51-C1-T4：含控制字符的版本值不得写入 resource（body 中出现了完整脏值）"
    );
    assert!(
        !body_injected.contains("9.9.9"),
        "PA51-C1-T4：脏值内的 `9.9.9` 片段不得泄漏进 wire"
    );
    assert!(
        body_injected.contains(env!("CARGO_PKG_VERSION")),
        "PA51-C1-T4：脏值必须下落 crate 版本 `{}`",
        env!("CARGO_PKG_VERSION")
    );

    // ② 超长（65 字节 > 上限 64）：不得写入，也不得静默截断成 64 字节后写入。
    let overlong = "9".repeat(65);
    let body_overlong = export_wire_body_with_explicit_version(Some(&overlong));
    assert!(
        !body_overlong.contains(&overlong),
        "PA51-C1-T4：超长（65 字节）版本值不得写入 resource"
    );
    assert!(
        !body_overlong.contains(&"9".repeat(64)),
        "PA51-C1-T4：超长值不得截断成 64 字节后写入（视为未设置，必须下落一级）"
    );
    assert!(
        body_overlong.contains(env!("CARGO_PKG_VERSION")),
        "PA51-C1-T4：超长值必须下落 crate 版本 `{}`",
        env!("CARGO_PKG_VERSION")
    );
}

/// PA51-C1-T3/T5：`PonySentryConfig::with_release` 三态 + trim 口径 + 脏值拒绝（纯构造器，无 IO）。
#[test]
fn c1_ponysentry_with_release() {
    let baseline = PonySentryConfig::default();
    assert_eq!(
        baseline.release,
        env!("CARGO_PKG_VERSION"),
        "PA51-C1-T5：缺省 release 必须等于 crate 版本（不含 env/IO）"
    );

    let explicit = PonySentryConfig::default().with_release(Some(" 9.9.9 "));
    assert_eq!(
        explicit.release, "9.9.9",
        "PA51-C1-T5：with_release 必须写入 trim 后的值"
    );
    assert_eq!(
        explicit.endpoint, baseline.endpoint,
        "PA51-C1-T3：with_release 不得改动其它字段（endpoint）"
    );
    assert_eq!(
        explicit.enabled, baseline.enabled,
        "PA51-C1-T3：with_release 不得改动其它字段（enabled）"
    );
    assert_eq!(
        explicit.client_token, baseline.client_token,
        "PA51-C1-T3：with_release 不得改动其它字段（client_token）"
    );

    let none = PonySentryConfig::default().with_release(None);
    assert_eq!(
        none.release, baseline.release,
        "PA51-C1-T3：`None` 必须保持原值（不得覆盖成空串）"
    );

    let blank = PonySentryConfig::default().with_release(Some("   "));
    assert_eq!(
        blank.release, baseline.release,
        "PA51-C1-T3：纯空白必须保持原值（不得覆盖成空串）"
    );

    let control = PonySentryConfig::default().with_release(Some("bad\u{0}ver"));
    assert_eq!(
        control.release, baseline.release,
        "PA51-C1-T4：含控制字符（NUL）的 release 必须保持原值（不得落非法字符）"
    );

    let overlong = "9".repeat(65);
    let truncated = PonySentryConfig::default().with_release(Some(&overlong));
    assert_eq!(
        truncated.release, baseline.release,
        "PA51-C1-T4：超长（65 字节）release 必须保持原值（不得截断写入）"
    );
}

// ============================================================ PA51-C2 · 治理会话归属

/// PA51-C2-T1：活跃 turn 内 `governance.decide` 必须携带 `langfuse.session.id`（会话过滤可命中）。
#[test]
fn c2_governance_span_carries_session() {
    let _guard = gate_lock();
    otel::set_reporting_enabled(false);
    otel::shutdown();

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(1_500));
    otel::init(local_config(port)).expect("本地回环端点必须可装配");
    otel::set_reporting_enabled(true);

    {
        let _turn = otel::begin_turn_trace("sess-gov", "turn-gov", 0);
        let mut governance = otel::start_governance_decide_span();
        governance.record_verdict("session", "allow", None, "pv-1", 0);
    }

    let flushed = otel::force_flush(3_000);
    assert!(flushed.is_ok(), "PA51-C2：flush 必须成功：{flushed:?}");
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(
        !captured.is_empty(),
        "PA51-C2-T1：必须至少导出一次 POST /v1/traces（捕获：{:?}）",
        describe(&captured)
    );

    // 关键：必须解码到 `governance.decide` **自身**属性再断言（Lead 裁决 v4+）。
    // 整段 body 的 `contains("langfuse.session.id")` 会被同一 trace 里 `agent.turn` span 的会话键
    // 满足 ⇒ 不可证伪的假绿；因此逐 span 取值，只认治理 span 自己的属性。
    let spans = decoded_spans(&captured);
    let governance = find_span(&spans, "governance.decide");
    assert_eq!(
        span_str_attribute(governance, "langfuse.session.id"),
        Some("sess-gov"),
        "PA51-C2-T1：活跃 turn 内的 `governance.decide` span 必须自带 `langfuse.session.id`，且与同 turn 逐字相同（期望 `sess-gov`）；治理 span 实际属性：{:?}",
        governance.attributes
    );
}

/// PA51-C2-T2：无活跃 turn ⇒ 治理 span **不得**输出 `langfuse.session.id`（禁止空串占位）。
#[test]
fn c2_governance_without_turn_has_no_session() {
    let _guard = gate_lock();
    otel::set_reporting_enabled(false);
    otel::shutdown();

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(1_500));
    otel::init(local_config(port)).expect("本地回环端点必须可装配");
    otel::set_reporting_enabled(true);

    {
        let mut governance = otel::start_governance_decide_span();
        governance.record_verdict("session", "allow", None, "pv-1", 0);
    }

    let flushed = otel::force_flush(3_000);
    assert!(flushed.is_ok(), "PA51-C2：flush 必须成功：{flushed:?}");
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(
        !captured.is_empty(),
        "PA51-C2-T2：必须至少导出一次 POST /v1/traces（捕获：{:?}）",
        describe(&captured)
    );
    let body = wire_body(&captured);

    assert!(
        body.contains("governance.decide"),
        "PA51-C2-T2：wire body 必须包含 `governance.decide` span（否则缺键断言自我满足）"
    );
    assert!(
        !body.contains("langfuse.session.id"),
        "PA51-C2-T2：无活跃 turn 时治理 span 不得输出 `langfuse.session.id`（含空串占位）"
    );
}

// ============================================================ PA51-C3 · 细分用量出口

const KEY_REASONING: &str = "gen_ai.usage.reasoning.output_tokens";
const KEY_CACHED: &str = "gen_ai.usage.cache_read.input_tokens";

/// PA51-C3-T1/T2/T3：纯函数值级契约（不依赖 wire varint 反解）。
#[test]
fn c3_usage_details_values() {
    // 常量字面量冻结：键名必须与契约/semconv 逐字一致（防「常量被改后自指自证」）。
    assert_eq!(
        ATTR_GEN_AI_USAGE_REASONING_TOKENS, KEY_REASONING,
        "PA51-C3：常量键名必须为 semconv `{KEY_REASONING}`"
    );
    assert_eq!(
        ATTR_GEN_AI_USAGE_CACHED_INPUT_TOKENS, KEY_CACHED,
        "PA51-C3：常量键名必须为 semconv `{KEY_CACHED}`"
    );

    // 有值才产出：`Some(v)` ⇒ 两键均为 AttrValue::I64。
    let both = otel::llm_usage_detail_attributes(Some(7), Some(5));
    assert_eq!(
        both.len(),
        2,
        "PA51-C3-T1：两值均存在时必须产出恰好 2 个键，实际 {both:?}"
    );
    assert_eq!(
        require_i64(&both, KEY_REASONING),
        7,
        "PA51-C3-T1：思考 token 必须原样输出"
    );
    assert_eq!(
        require_i64(&both, KEY_CACHED),
        5,
        "PA51-C3-T1：缓存命中输入 token 必须原样输出"
    );

    // 两键互相独立：可只出现其一。
    let only_reasoning = otel::llm_usage_detail_attributes(Some(3), None);
    assert_eq!(
        only_reasoning.len(),
        1,
        "PA51-C3-T1：仅思考 token 有值时必须只产出 1 个键，实际 {only_reasoning:?}"
    );
    assert_eq!(require_i64(&only_reasoning, KEY_REASONING), 3);
    assert!(
        attr(&only_reasoning, KEY_CACHED).is_none(),
        "PA51-C3-T1：缓存键无值不得产出"
    );

    let only_cached = otel::llm_usage_detail_attributes(None, Some(4));
    assert_eq!(
        only_cached.len(),
        1,
        "PA51-C3-T1：仅缓存有值时必须只产出 1 个键，实际 {only_cached:?}"
    );
    assert_eq!(require_i64(&only_cached, KEY_CACHED), 4);
    assert!(
        attr(&only_cached, KEY_REASONING).is_none(),
        "PA51-C3-T1：思考键无值不得产出"
    );

    // 缺失 ≠ 0：both None ⇒ 零键（禁止 0 占位）。
    let absent = otel::llm_usage_detail_attributes(None, None);
    assert!(
        absent.is_empty(),
        "PA51-C3-T2：两值均缺失时不得产出任何键（含 0 占位），实际 {absent:?}"
    );

    // `Some(0)` 是有效取值：必须原样输出 0。
    let zeros = otel::llm_usage_detail_attributes(Some(0), Some(0));
    assert_eq!(
        zeros.len(),
        2,
        "PA51-C3-T2：`Some(0)` 是有效采集结果，必须输出两键，实际 {zeros:?}"
    );
    assert_eq!(require_i64(&zeros, KEY_REASONING), 0, "PA51-C3-T2：0 必须原样输出");
    assert_eq!(require_i64(&zeros, KEY_CACHED), 0, "PA51-C3-T2：0 必须原样输出");

    // `u64::MAX` ⇒ 安全饱和到 i64::MAX（禁止 `as` 直转成负数/其它值）。
    let saturated = otel::llm_usage_detail_attributes(Some(u64::MAX), Some(u64::MAX));
    assert_eq!(
        require_i64(&saturated, KEY_REASONING),
        i64::MAX,
        "PA51-C3-T3：超 i64::MAX 的 u64 必须饱和为 i64::MAX（不得为负/截断）"
    );
    assert_eq!(
        require_i64(&saturated, KEY_CACHED),
        i64::MAX,
        "PA51-C3-T3：超 i64::MAX 的 u64 必须饱和为 i64::MAX（不得为负/截断）"
    );
    assert_eq!(
        require_i64(&saturated, KEY_REASONING),
        9_223_372_036_854_775_807,
        "PA51-C3-T3：饱和值必须是字面量 9223372036854775807"
    );
}

/// PA51-C3-T4：wire 级必须出现两键；根 `agent.turn` 聚合口径用纯函数钉住（wire 不可反解整数）。
#[test]
fn c3_wire_keys_and_root_aggregation() {
    let _guard = gate_lock();
    otel::set_reporting_enabled(false);
    otel::shutdown();

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(2_000));
    otel::init(local_config(port)).expect("本地回环端点必须可装配");
    otel::set_reporting_enabled(true);

    {
        let _turn = otel::begin_turn_trace("sess-usage", "turn-usage", 0);

        let mut first = otel::start_llm_span("mock-provider", Some("gpt-4o"), "chat");
        first.record_usage(Some(100), Some(50));
        first.record_usage_details(Some(7), Some(5));
        drop(first);

        let mut second = otel::start_llm_span("mock-provider", Some("gpt-4o"), "chat");
        second.record_usage(Some(80), Some(40));
        second.record_usage_details(Some(3), Some(2));
        drop(second);
    }

    let flushed = otel::force_flush(3_000);
    assert!(flushed.is_ok(), "PA51-C3：flush 必须成功：{flushed:?}");
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(
        !captured.is_empty(),
        "PA51-C3-T4：必须至少导出一次 POST /v1/traces（捕获：{:?}）",
        describe(&captured)
    );
    let body = wire_body(&captured);

    assert!(
        body.contains(KEY_REASONING),
        "PA51-C3-T4：`llm.chat` / 根 `agent.turn` 必须导出 `{KEY_REASONING}` 键"
    );
    assert!(
        body.contains(KEY_CACHED),
        "PA51-C3-T4：`llm.chat` / 根 `agent.turn` 必须导出 `{KEY_CACHED}` 键"
    );

    // 子 span 值级断言（wire varint 反解）：两次调用分别是 (7,5) 与 (3,2)。
    let spans = decoded_spans(&captured);
    let mut reasoning_values: Vec<i64> = Vec::new();
    let mut cached_values: Vec<i64> = Vec::new();
    for span in spans.iter().filter(|span| span.name == "llm.chat") {
        if let Some(value) = span_int_attribute(span, KEY_REASONING) {
            reasoning_values.push(value);
        }
        if let Some(value) = span_int_attribute(span, KEY_CACHED) {
            cached_values.push(value);
        }
    }
    reasoning_values.sort_unstable();
    cached_values.sort_unstable();
    assert_eq!(
        reasoning_values,
        vec![3, 7],
        "PA51-C3-T4：两个 `llm.chat` 子 span 必须各自导出自己的思考 token（7 / 3），实际 {reasoning_values:?}"
    );
    assert_eq!(
        cached_values,
        vec![2, 5],
        "PA51-C3-T4：两个 `llm.chat` 子 span 必须各自导出自己的缓存命中（5 / 2），实际 {cached_values:?}"
    );

    // 根 `agent.turn` 聚合值级断言：Σ=10 / Σ=7（不是「键存在」即放行）。
    let root = find_span(&spans, "agent.turn");
    assert_eq!(
        span_int_attribute(root, KEY_REASONING),
        Some(10),
        "PA51-C3-T4：根 `agent.turn` 的 `{KEY_REASONING}` 必须为 Σ=7+3=10"
    );
    assert_eq!(
        span_int_attribute(root, KEY_CACHED),
        Some(7),
        "PA51-C3-T4：根 `agent.turn` 的 `{KEY_CACHED}` 必须为 Σ=5+2=7"
    );

    // 纯函数口径一致性：根聚合与子 span 共用同一份映射（Σ=10/7）。
    let aggregate = otel::llm_usage_detail_attributes(Some(7 + 3), Some(5 + 2));
    assert_eq!(
        aggregate.len(),
        2,
        "PA51-C3-T4：根聚合键集必须恰好两个（不得夹带其它键），实际 {aggregate:?}"
    );
    assert_eq!(
        require_i64(&aggregate, KEY_REASONING),
        10,
        "PA51-C3-T4：纯函数层 Σ 口径必须与根 span 一致"
    );
    assert_eq!(
        require_i64(&aggregate, KEY_CACHED),
        7,
        "PA51-C3-T4：纯函数层 Σ 口径必须与根 span 一致"
    );
}

/// PA51-C3-T5：所有子 span 都不上报细分用量 ⇒ 根 `agent.turn` 不得输出两键（禁止 0 占位）。
#[test]
fn c3_all_none_root_has_no_keys() {
    let _guard = gate_lock();
    otel::set_reporting_enabled(false);
    otel::shutdown();

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(1_500));
    otel::init(local_config(port)).expect("本地回环端点必须可装配");
    otel::set_reporting_enabled(true);

    {
        let _turn = otel::begin_turn_trace("sess-usage-none", "turn-usage-none", 0);
        let mut span = otel::start_llm_span("mock-provider", Some("gpt-4o"), "chat");
        span.record_usage(Some(100), Some(50));
    }

    let flushed = otel::force_flush(3_000);
    assert!(flushed.is_ok(), "PA51-C3：flush 必须成功：{flushed:?}");
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(
        !captured.is_empty(),
        "PA51-C3-T5：必须至少导出一次 POST /v1/traces（捕获：{:?}）",
        describe(&captured)
    );
    let body = wire_body(&captured);

    // 正向对照：同一次导出必须带 input/output 键，否则「缺键」可能只是没导出。
    assert!(
        body.contains("gen_ai.usage.input_tokens"),
        "PA51-C3-T5：既有 `gen_ai.usage.input_tokens` 出口不得被破坏（否则缺键断言不可信）"
    );
    assert!(
        !body.contains(KEY_REASONING),
        "PA51-C3-T5：全部子 span 未上报思考 token 时不得输出 `{KEY_REASONING}`（禁止 0 占位）"
    );
    assert!(
        !body.contains(KEY_CACHED),
        "PA51-C3-T5：全部子 span 未上报缓存命中时不得输出 `{KEY_CACHED}`（禁止 0 占位）"
    );

    // 解码级复核：根 span 上两键必须缺席（缺键 ≠ 0）。
    let spans = decoded_spans(&captured);
    let root = find_span(&spans, "agent.turn");
    assert_eq!(
        span_int_attribute(root, KEY_REASONING),
        None,
        "PA51-C3-T5：根 `agent.turn` 不得以 0 伪造 `{KEY_REASONING}`"
    );
    assert_eq!(
        span_int_attribute(root, KEY_CACHED),
        None,
        "PA51-C3-T5：根 `agent.turn` 不得以 0 伪造 `{KEY_CACHED}`"
    );
}

// ============================================================ T6 · 静态接线断言

/// PA51-C3-T6：4 个 `record_usage` 调用点旁必须各接一次 `record_usage_details`；宿主装配入口已切换
/// 并注入 PonySentry `release`。
#[test]
fn t6_static_wiring() {
    let turn_flow_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src")
        .join("agent")
        .join("turn_flow.rs");
    let turn_flow = std::fs::read_to_string(&turn_flow_path)
        .unwrap_or_else(|err| panic!("PA51-C3-T6：必须能读到 {}：{err}", turn_flow_path.display()));

    let detail_calls = turn_flow.matches("record_usage_details(").count();
    assert_eq!(
        detail_calls, 4,
        "PA51-C3-T6：`turn_flow.rs` 必须恰好 4 处 `record_usage_details(` 接线（不得漏接/多接），实际 {detail_calls}"
    );

    let host_lib_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("src-tauri")
        .join("src")
        .join("lib.rs");
    let host_lib = std::fs::read_to_string(&host_lib_path)
        .unwrap_or_else(|err| panic!("PA51-C1：必须能读到宿主装配点 {}：{err}", host_lib_path.display()));

    assert!(
        host_lib.contains("init_with_service_version("),
        "PA51-C1：宿主必须用 `init_with_service_version(` 装配 OTel（安装包版本注入通道）"
    );
    assert!(
        host_lib.contains(".with_release("),
        "PA51-C1：宿主必须用 `.with_release(` 注入 PonySentry release"
    );
}

// ============================================================ L2-AT · 对抗

/// L2-AT：两个线程各做 3 轮 `init/export`，`Barrier` 对齐起跑（确定性竞态，非 `sleep` 盲猜）。
///
/// 契约：允许 `Err`（`init` 失败只降级不阻断），但**不得 panic**、不得死锁、不得因半初始化崩栈。
#[test]
fn l2at_concurrent_init() {
    let _guard = gate_lock();
    otel::set_reporting_enabled(false);
    otel::shutdown();

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(1_500));
    let barrier = Arc::new(Barrier::new(3));

    fn worker(worker_id: usize, port: u16, barrier: Arc<Barrier>) -> Option<String> {
        for round in 0..3u64 {
            barrier.wait();
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                otel::shutdown();
                // `Err` 是允许的降级结果（并发下装配必然有且仅有一次成功）。
                if otel::init_with_service_version(local_config(port), None).is_ok() {
                    otel::set_reporting_enabled(true);
                    {
                        let _turn =
                            otel::begin_turn_trace("sess-l2at", "turn-l2at", round);
                        let mut span = otel::start_llm_span("mock-provider", Some("gpt-4o"), "chat");
                        span.record_usage(Some(10), Some(5));
                    }
                    let _ = otel::force_flush(500);
                    otel::shutdown();
                    otel::set_reporting_enabled(false);
                }
            }));
            if outcome.is_err() {
                return Some(format!(
                    "worker {worker_id} 第 {round} 轮 init/export 发生 panic（并发下只允许返回 Err）"
                ));
            }
        }
        None
    }

    let handles: Vec<JoinHandle<Option<String>>> = (0..2usize)
        .map(|worker_id| {
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || worker(worker_id, port, barrier))
        })
        .collect();

    // 主线程也参与 Barrier，让两线程在 init 前对齐（确定性起跑）。
    // TEST-DEFECT 修复（Lead 裁决，绿相发现）：`Barrier::new(3)` 的参与方是「主线程 + 2 worker」，
    // worker 每轮各 wait 一次 ⇒ 主线程必须与 worker 同步 **3 轮**（原先只 wait 一次，第 2 轮必挂死）。
    for _ in 0..3 {
        barrier.wait();
    }

    let mut failures: Vec<String> = Vec::new();
    for handle in handles {
        match handle.join() {
            Ok(None) => {}
            Ok(Some(message)) => failures.push(message),
            Err(_) => failures.push("worker 线程在 catch_unwind 之外 panic".to_string()),
        }
    }
    assert!(
        failures.is_empty(),
        "L2-AT：并发 `init_with_service_version` + export 不得 panic（允许 Err），实际：{failures:?}"
    );

    otel::shutdown();
    otel::set_reporting_enabled(false);
    // 回收 sink 线程（硬截止预算内结束，避免用例间残留监听）。
    let captured = sink.join().expect("sink 线程不得 panic");
    let _ = describe(&captured);
}
