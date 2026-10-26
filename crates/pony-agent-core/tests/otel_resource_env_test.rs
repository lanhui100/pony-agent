//! Wave-45 · PA-153 T2 验收测试（红相）：resource 归因的**进程 env 覆盖面**。
//!
//! 契约来源（唯一口径）：`.dev-team/contract-matrix-wave45.md` §1 PA-153 T2 行 + §0 F3/F15：
//! - `PONY_OTEL_SERVICE_VERSION` / `PONY_OTEL_ENVIRONMENT` trim 后非空 ⇒ 覆盖默认值；
//! - 空白串等同未设置 ⇒ 回落默认（core crate 版本 / debug 构建 `development`）；
//! - `OtelConfig` 4 公开字段与 7 处字面构造点零改动（故覆盖只能经进程 env，不能经 config 字段）。
//!
//! 为什么**单独一个测试二进制**（Lead 裁决）：
//! 覆盖能力只能经进程 env 触发，而 env 是进程级全局态；与 `otel_resource_turn_status_test.rs`
//! 的 resource 断言同进程会互相串味（后者断言默认版本 / `development`）。本文件因此进程隔离，
//! 并且**只用一个 `#[test]` 顺序驱动两个环境相位**（`init` → flush → `shutdown` → 再 `init`），
//! 保证「同一时刻只有一个 provider 与一个确定 env 态」。
//!
//! 断言纪律：只引用 HEAD 已存在公开 API（`otel::init` / `begin_turn_trace` / `force_flush` /
//! `shutdown` / `set_reporting_enabled` / `OtelConfig` 字面构造），未实现时必须是**断言失败**
//! 而非编译失败；wire 断言一律 ASCII 子串（不解析 protobuf）；无 `#[ignore]`、无无界等待。
//!
//! 作者：test-agent-45（L2-AT）。红相锚定后本文件冻结只读，任何修改须 Lead 批准。

use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use pony_agent_core::agent::otel::{self, DEFAULT_SAMPLING_RATIO};

// -------------------------------------------------- 本地 OTLP/HTTP sink（探针）

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
        service_name: "pony-agent-wave45-env-acceptance".to_string(),
        sampling_ratio: DEFAULT_SAMPLING_RATIO,
        headers: Vec::new(),
    }
}

/// 跑一个环境相位：起 sink → init → 开 1 个 turn → flush → shutdown → 返回 wire 文本。
fn run_phase(tag: &str) -> String {
    otel::shutdown();
    otel::set_reporting_enabled(false);
    let (port, sink) = spawn_otlp_sink(Duration::from_millis(2_000));
    otel::init(local_config(port)).expect("本地回环端点必须可装配");
    otel::set_reporting_enabled(true);

    {
        let mut turn = otel::begin_turn_trace(
            &format!("sess-pa153-t2-{tag}"),
            &format!("turn-pa153-t2-{tag}"),
            1,
        );
        turn.set_phase("completed");
    }

    let flushed = otel::force_flush(3_000);
    assert!(flushed.is_ok(), "已 init 时 force_flush 必须返回 Ok：{flushed:?}");
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(
        !captured.is_empty(),
        "[{tag}] 本相位必须至少收到一个 OTLP 导出请求（否则后续 wire 断言全是假绿）"
    );
    captured
        .iter()
        .map(|request| {
            assert_eq!(request.method, "POST", "[{tag}] OTLP/HTTP 导出必须为 POST");
            assert!(
                request.path.contains("/v1/traces"),
                "[{tag}] 导出路径必须为 /v1/traces，实际 `{}`",
                request.path
            );
            String::from_utf8_lossy(&request.body).to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 诊断小工具：截断 + 非可打印字符替换（避免把整包 protobuf 吐到 CI 日志）。
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

// ============================================================================ PA-153 T2

#[test]
fn pa153_t2_env_overrides_service_version_and_environment() {
    // 契约 §1 T2 冻结的字面量（防「env 名被改名后自指自证」）。
    assert_eq!("PONY_OTEL_SERVICE_VERSION", "PONY_OTEL_SERVICE_VERSION");
    assert_eq!("PONY_OTEL_ENVIRONMENT", "PONY_OTEL_ENVIRONMENT");

    let default_version = env!("CARGO_PKG_VERSION");

    // ---- 相位 A：两个 env 哨兵值必须覆盖默认值 ----
    std::env::set_var("PONY_OTEL_SERVICE_VERSION", "9.9.9-wave45");
    std::env::set_var("PONY_OTEL_ENVIRONMENT", "staging");

    let overridden = run_phase("override");

    // 非空洞锚：body 必须真的承载本轮 span。
    assert!(
        overridden.contains("agent.turn"),
        "PA153-T2 非空洞锚：body 必须承载 agent.turn，片段：{}",
        truncate_for_message(&overridden)
    );
    assert!(
        overridden.contains("service.version"),
        "PA153-T2 前置：resource 必须先有 `service.version` 键（PA153-T1 冻结项），片段：{}",
        truncate_for_message(&overridden)
    );
    assert!(
        overridden.contains("deployment.environment"),
        "PA153-T2 前置：resource 必须先有 `deployment.environment` 键（PA153-T1 冻结项），片段：{}",
        truncate_for_message(&overridden)
    );

    // Assert A1：service.version 被 env 覆盖。
    assert!(
        overridden.contains("9.9.9-wave45"),
        "F3/PA153-T2：`PONY_OTEL_SERVICE_VERSION` 非空时必须覆盖 service.version 为 `9.9.9-wave45`，片段：{}",
        truncate_for_message(&overridden)
    );
    // Assert A2：deployment.environment 被 env 覆盖。
    assert!(
        overridden.contains("staging"),
        "F3/PA153-T2：`PONY_OTEL_ENVIRONMENT` 非空时必须覆盖 deployment.environment 为 `staging`，片段：{}",
        truncate_for_message(&overridden)
    );
    // Assert A3：覆盖生效时不得同时输出默认版本（避免「双写」造成版本归因歧义）。
    assert!(
        !overridden.contains(default_version),
        "PA153-T2：覆盖生效时不得同时输出默认版本 `{default_version}`，片段：{}",
        truncate_for_message(&overridden)
    );

    // ---- 相位 B：空白串等同未设置 ⇒ 回落默认 ----
    std::env::set_var("PONY_OTEL_SERVICE_VERSION", "   ");
    std::env::set_var("PONY_OTEL_ENVIRONMENT", "  ");

    let fallback = run_phase("blank");

    assert!(
        fallback.contains("agent.turn"),
        "PA153-T2 非空洞锚（空白相位）：body 必须承载 agent.turn，片段：{}",
        truncate_for_message(&fallback)
    );
    // Assert B1：空白串 ⇒ 回落默认版本。
    assert!(
        fallback.contains(default_version),
        "PA153-T2：空白串必须等同未设置（trim 后判空）⇒ 回落默认版本 `{default_version}`，片段：{}",
        truncate_for_message(&fallback)
    );
    // Assert B2：空白串 ⇒ 回落 debug 默认环境（release 分支由实现期纯函数单测覆盖，契约 §1 第 3 行）。
    if cfg!(debug_assertions) {
        assert!(
            fallback.contains("development"),
            "PA153-T2：空白串必须回落 debug 默认环境 `development`，片段：{}",
            truncate_for_message(&fallback)
        );
    }
    // Assert B3：空白串不得残留为实际取值。
    assert!(
        !fallback.contains("9.9.9-wave45") && !fallback.contains("staging"),
        "PA153-T2：空白串不得残留为实际取值，片段：{}",
        truncate_for_message(&fallback)
    );

    // 退出恢复 env（谁修改谁恢复；本文件是独立进程，仍按纪律复原以便重复运行）。
    std::env::remove_var("PONY_OTEL_SERVICE_VERSION");
    std::env::remove_var("PONY_OTEL_ENVIRONMENT");
}
