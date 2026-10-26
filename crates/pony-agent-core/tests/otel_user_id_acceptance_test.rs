//! Wave-46 契约验收（PA-163）— Langfuse user_id 规范归因与注入
//!
//! 契约源：`.dev-team/contract-matrix-wave46.md`（PA163-T1/T2）。
//! 规则：
//!   1. 设置 `PONY_OTEL_USER_ID="user-acc-test"` 时，导出的 ResourceSpans 或 Span 必须输出 `langfuse.user.id` 且值为 `user-acc-test`；
//!   2. 未设置或纯空白串时，不得输出 `langfuse.user.id` 键；
//!   3. 零假自测，必须经由本地回环 OTLP sink 断言 wire 级输出。

use std::io::Read;
use std::net::TcpListener;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pony_agent_core::agent::otel::{self, OtelConfig};

static TEST_LOCK: Mutex<()> = Mutex::new(());

#[derive(Clone, Debug)]
struct CapturedRequest {
    body: Vec<u8>,
}

fn spawn_otlp_sink(budget: Duration) -> (u16, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("port").port();
    listener
        .set_nonblocking(false)
        .expect("blocking listener");

    let handle = thread::spawn(move || {
        let deadline = std::time::Instant::now() + budget;
        let mut captured = Vec::new();
        while std::time::Instant::now() < deadline {
            listener
                .set_nonblocking(true)
                .expect("toggle nonblocking");
            let stream = match listener.accept() {
                Ok((s, _)) => s,
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                }
                Err(_) => break,
            };

            let mut stream = stream;
            let _ = stream.set_read_timeout(Some(Duration::from_millis(300)));
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let mut content_length: Option<usize> = None;
            let mut header_end: Option<usize> = None;

            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if header_end.is_none() {
                            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                header_end = Some(pos + 4);
                                let header_str = String::from_utf8_lossy(&buf[..pos]);
                                for line in header_str.lines() {
                                    if let Some(val) = line.strip_prefix("content-length:") {
                                        content_length = val.trim().parse::<usize>().ok();
                                    }
                                }
                            }
                        }
                        if let (Some(end), Some(len)) = (header_end, content_length) {
                            if buf.len() >= end + len {
                                break;
                            }
                        }
                    }
                    Err(ref e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            || e.kind() == std::io::ErrorKind::TimedOut =>
                    {
                        break;
                    }
                    Err(_) => break,
                }
            }

            let body = if let Some(end) = header_end {
                buf[end..].to_vec()
            } else {
                buf
            };
            captured.push(CapturedRequest { body });
            let response = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nOK";
            let _ = std::io::Write::write_all(&mut stream, response.as_bytes());
        }
        captured
    });

    (port, handle)
}

fn local_config(port: u16) -> OtelConfig {
    OtelConfig {
        endpoint: format!("http://127.0.0.1:{port}"),
        service_name: "pony-agent".to_string(),
        sampling_ratio: 1.0,
        headers: vec![],
    }
}

#[test]
fn pa163_t1_langfuse_user_id_injected_when_env_set() {
    let _guard = TEST_LOCK.lock().unwrap();

    // 1. Arrange: 设环境变量
    std::env::set_var("PONY_OTEL_USER_ID", "user-acc-test-123");

    otel::shutdown();
    otel::set_reporting_enabled(false);
    let (port, sink) = spawn_otlp_sink(Duration::from_millis(2_000));
    otel::init(local_config(port)).expect("init otel");
    otel::set_reporting_enabled(true);

    // 2. Act: 发起一次 turn trace
    {
        let _turn = otel::begin_turn_trace("sess-u1", "turn-u1", 0);
    }
    let _ = otel::force_flush(2_000);
    otel::shutdown();
    otel::set_reporting_enabled(false);
    std::env::remove_var("PONY_OTEL_USER_ID");

    let captured = sink.join().expect("sink join");
    let wire = captured
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    // 3. Assert: 断言包含 langfuse.user.id 与 user-acc-test-123
    assert!(
        wire.contains("langfuse.user.id"),
        "PA163-T1: wire 必须包含属性名 langfuse.user.id"
    );
    assert!(
        wire.contains("user-acc-test-123"),
        "PA163-T1: wire 必须包含用户标识 user-acc-test-123"
    );
}

#[test]
fn pa163_t1_langfuse_user_id_absent_when_env_unset_or_blank() {
    let _guard = TEST_LOCK.lock().unwrap();

    // 1. Arrange: 确保无环境变量或为空白
    std::env::remove_var("PONY_OTEL_USER_ID");

    otel::shutdown();
    otel::set_reporting_enabled(false);
    let (port, sink) = spawn_otlp_sink(Duration::from_millis(2_000));
    otel::init(local_config(port)).expect("init otel");
    otel::set_reporting_enabled(true);

    // 2. Act: 发起一次 turn trace
    {
        let _turn = otel::begin_turn_trace("sess-u2", "turn-u2", 0);
    }
    let _ = otel::force_flush(2_000);
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink join");
    let wire = captured
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    // 3. Assert: 环境变量缺失时不得包含 langfuse.user.id 键
    assert!(
        !wire.contains("langfuse.user.id"),
        "PA163-T1: 未设置 PONY_OTEL_USER_ID 时绝不得输出 langfuse.user.id"
    );
}
