//! r2-dim1（维度 1 时延结构）· L3 验收测试：无首包调用必须自带传输口径。
//!
//! 缺陷事实（`docs/audit/rounds/r2-dim1/root-cause.md` + 提案 F3）：`llm.request.mode` 只表达
//! 「请求在轮次里的角色」（initial / tool_followup / tool_recovery），对流式与缓冲请求都写
//! `tool_followup` ⇒ 下游无法区分「批量请求本就没有首包」与「流式首包测量丢包」。
//! 实测 18/18 会话的 `ttft_missing_calls` 恒等于 buffered HTTP 调用数，但 trace 上没有任何
//! 键可以证明这一点。
//!
//! 覆盖：
//! - **RT-T1**：`llm.request.transport` 上 wire，值为 `stream` / `buffered`，与
//!   `llm.request.mode` / `messages` / `system_prefix` 同 span 齐备（四键同生同灭）。
//! - **RT-T2**：`buffered` span **无** `llm.call.ttft_ms`，而 `stream` span 有 —— 即
//!   「无首包」在 wire 上可与「流式丢包」区分（L3 断言的语义前提）。
//!
//! 纪律（与 `wave54_metric_truth.rs` 同构）：零外网回环 OTLP sink、属性键写字面量、
//! 禁止 `#[ignore]`、禁止无界等待、全局门禁串行锁。

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::otel::{self, DEFAULT_SAMPLING_RATIO};

static GATE_LOCK: Mutex<()> = Mutex::new(());

fn gate_lock() -> MutexGuard<'static, ()> {
    GATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

const KEY_MODE: &str = "llm.request.mode";
const KEY_MESSAGES: &str = "llm.request.messages";
const KEY_SYSTEM_PREFIX: &str = "llm.request.system_prefix";
const KEY_TRANSPORT: &str = "llm.request.transport";
const KEY_TTFT: &str = "llm.call.ttft_ms";
const KEY_DURATION: &str = "llm.call.duration_ms";
const KEY_REQUEST_MODEL: &str = "gen_ai.request.model";

const MODEL_STREAM: &str = "r2dim1-rt-stream";
const MODEL_BUFFERED: &str = "r2dim1-rt-buffered";

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
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
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
        service_name: "pony-agent-r2dim1-transport".to_string(),
        sampling_ratio: DEFAULT_SAMPLING_RATIO,
        headers: Vec::new(),
    }
}

/// 导出全部 `POST /v1/traces` body 拼成的 wire 文本（ASCII 键/值可直接子串匹配）。
fn wire_text(captured: &[CapturedRequest]) -> String {
    assert!(
        !captured.is_empty(),
        "本次窗口内必须至少收到一个 OTLP 导出请求（否则后续 wire 断言全部是假绿）"
    );
    captured
        .iter()
        .map(|request| {
            assert_eq!(request.method, "POST", "OTLP/HTTP 导出必须为 POST");
            assert!(
                request.path.contains("/v1/traces"),
                "OTLP/HTTP trace 导出路径必须为 /v1/traces，实际 `{}`",
                request.path
            );
            String::from_utf8_lossy(&request.body).to_string()
        })
        .collect::<Vec<String>>()
        .join("\n")
}

/// 断言某个字符串属性的**值**以 AnyValue 编码上 wire。
///
/// 为什么要按编码断言而不是「键后窗口 contains 值」：同一份 wire 里有多个 span，
/// 首处出现的键未必是目标 span；而 `AnyValue.string_value` 的字节形态是确定的
/// （`KeyValue.value` 字段 2 → `AnyValue.string_value` 字段 1 + 长度 + UTF-8 字节），
/// 因此「值 + 正确长度前缀」这一组合只可能来自该键本身。
fn assert_any_value_str(wire: &str, key: &str, expected: &str) {
    let bytes = wire.as_bytes();
    let mut cursor = 0usize;
    let mut hits = 0usize;
    while let Some(found) = find_subslice(&bytes[cursor..], key.as_bytes()) {
        let after = cursor + found + key.len();
        // KeyValue.value（字段 2，wire type 2）→ AnyValue.string_value（字段 1，wire type 2）。
        if after + 2 <= bytes.len() && bytes[after] == 0x12 {
            let inner = after + 1;
            let length = bytes[inner] as usize;
            let start = inner + 1;
            if start + 2 <= bytes.len()
                && bytes[start] == 0x0a
                && bytes[start + 1] as usize == expected.len()
                && start + 2 + expected.len() <= bytes.len()
                && &bytes[start + 2..start + 2 + expected.len()] == expected.as_bytes()
            {
                assert_eq!(
                    length,
                    expected.len() + 2,
                    "`{key}` 的 AnyValue 长度前缀必须等于 `{expected}` 的编码长度"
                );
                hits += 1;
            }
        }
        cursor = after;
    }
    assert!(
        hits > 0,
        "`{key}` 必须以字符串值 `{expected}` 上 wire（AnyValue.string_value 编码未命中）"
    );
}

/// 一次完整导出周期：复位 → 回环 sink → init → 开门禁 → 造 span → force_flush → shutdown → 解码。
fn run_export<F: FnOnce()>(budget: Duration, body: F) -> String {
    otel::set_reporting_enabled(false);
    otel::shutdown();

    let (port, sink) = spawn_otlp_sink(budget);
    let assembled = otel::init(local_config(port));
    assert!(
        assembled.is_ok(),
        "本地回环端点必须可装配（装配失败只降级不 panic）：{assembled:?}"
    );
    otel::set_reporting_enabled(true);

    body();

    let flushed = otel::force_flush(3_000);
    assert!(
        flushed.is_ok(),
        "已装配时 force_flush 必须成功（否则缺键断言可能只是「没导出」）：{flushed:?}"
    );
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink 线程不得 panic");
    wire_text(&captured)
}

/// RT-T1 / RT-T2：两种传输方式各自的四键与首包事实。
#[test]
fn llm_request_transport_distinguishes_stream_from_buffered() {
    let _guard = gate_lock();

    let wire = run_export(Duration::from_millis(2_500), || {
        let _turn = otel::begin_turn_trace("sess-r2dim1-rt", "turn-r2dim1-rt", 1);

        // ① 流式调用：有首包。
        let mut stream = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(MODEL_STREAM),
            "chat",
        );
        stream.record_request_shape(
            "tool_followup",
            3,
            true,
            otel::LLM_REQUEST_TRANSPORT_STREAM,
        );
        stream.record_call_timing(Some(1_200), Some(200));
        drop(stream);

        // ② 缓冲调用：本就没有首包语义（不是「测量丢包」）。
        let mut buffered = otel::start_llm_span_with_provider(
            "openai-completions",
            Some("PonyLlm"),
            Some(MODEL_BUFFERED),
            "chat",
        );
        buffered.record_request_shape(
            "tool_followup",
            3,
            true,
            otel::LLM_REQUEST_TRANSPORT_BUFFERED,
        );
        buffered.record_call_timing(Some(900), None);
        drop(buffered);
    });

    // 正向锚：两个 span 都确实导出（否则「键缺失」断言无意义）。
    assert!(
        wire.contains(MODEL_STREAM),
        "wire 必须含 stream 用例的 `{KEY_REQUEST_MODEL}` 值（证明 span 已导出）"
    );
    assert!(
        wire.contains(MODEL_BUFFERED),
        "wire 必须含 buffered 用例的 `{KEY_REQUEST_MODEL}` 值（证明 span 已导出）"
    );

    // RT-T1：四键齐备，transport 取值精确。
    assert_eq!(
        wire.matches(KEY_TRANSPORT).count(),
        2,
        "两个 `llm.chat` span 各须输出一次 `{KEY_TRANSPORT}`（四键同生同灭）"
    );
    assert_any_value_str(&wire, KEY_TRANSPORT, "stream");
    assert_any_value_str(&wire, KEY_TRANSPORT, "buffered");
    assert_eq!(
        wire.matches(KEY_MODE).count(),
        2,
        "既有 `{KEY_MODE}` 不得因新增键而消失"
    );
    assert_eq!(wire.matches(KEY_MESSAGES).count(), 2, "既有 `{KEY_MESSAGES}` 不得消失");
    assert_eq!(
        wire.matches(KEY_SYSTEM_PREFIX).count(),
        2,
        "既有 `{KEY_SYSTEM_PREFIX}` 不得消失"
    );

    // RT-T2：只有流式 span 有首包；缓冲 span 的 duration 仍在（真值不退化）。
    assert_eq!(
        wire.matches(KEY_TTFT).count(),
        1,
        "`{KEY_TTFT}` 只应由流式调用输出（缓冲调用无首包语义）"
    );
    assert_eq!(
        wire.matches(KEY_DURATION).count(),
        2,
        "两次调用的 `{KEY_DURATION}` 都必须输出（缓冲调用不得丢耗时真值）"
    );
}

/// RT-T1 补充：常量字面量冻结（防「常量被改名后自指自证」）。
#[test]
fn transport_key_and_values_are_frozen_literals() {
    assert_eq!(otel::ATTR_LLM_REQUEST_TRANSPORT, "llm.request.transport");
    assert_eq!(otel::LLM_REQUEST_TRANSPORT_STREAM, "stream");
    assert_eq!(otel::LLM_REQUEST_TRANSPORT_BUFFERED, "buffered");
}
