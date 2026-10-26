//! Wave-47 · ob2 收口点实时链路验收：`http_client` 的 `traceparent` 保证
//!
//! 目的：本轮 `http_client` 收口点声明的保证是「出网请求带 W3C traceparent」。
//! 单测只校验 `HeaderMap` 层面的拼装；本套件走**真实 TCP 套接字**，捕获 reqwest 实际
//! 写出的 HTTP/1.1 原始字节，断言头真的上了线。
//!
//! 双模态：
//! - L2-T（正相）：活跃 turn 内 → 线上必须出现 `traceparent: 00-<32hex>-<16hex>-01`
//! - L2-AT（反相对照）：无活跃 turn → 线上**不得**出现 traceparent（证明头来自收口点注入，
//!   而非 reqwest 默认值或测试夹具污染），这是区分「真注入」与「假绿」的鉴别力断言。

use pony_agent_core::agent::otel::{begin_turn_trace, with_current_traceparent};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::future::Future;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// 同步驱动 async 请求（crate 未启用 tokio `macros` feature，故不用 `#[tokio::test]`）。
fn block_on<F: Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("构建 tokio runtime 失败")
        .block_on(future)
}

/// 在 127.0.0.1 起一个只接一次连接的裸 HTTP 服务，回传请求的原始字节。
fn spawn_raw_capture_server() -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定本地端口失败");
    let addr = listener.local_addr().expect("取本地地址失败");
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("设置读超时失败");
            let mut buf = Vec::new();
            let mut chunk = [0u8; 1024];
            // 读到请求头结束为止（\r\n\r\n），GET 无 body，足够拿到全部头部。
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        buf.extend_from_slice(&chunk[..n]);
                        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Type: application/json\r\n\r\n{}",
            );
            let _ = stream.flush();
            let _ = tx.send(String::from_utf8_lossy(&buf).to_string());
        }
    });

    (format!("http://{addr}/v1/probe"), rx)
}

/// 从原始请求文本中取出 traceparent 头的值（不存在则 None）。
fn extract_traceparent(raw: &str) -> Option<String> {
    raw.lines().find_map(|line| {
        let lower = line.to_ascii_lowercase();
        lower
            .starts_with("traceparent:")
            .then(|| line["traceparent:".len()..].trim().to_string())
    })
}

/// W3C 形态校验：`00-<32位小写hex>-<16位小写hex>-<2位hex>`
fn is_w3c_traceparent(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    parts.len() == 4
        && parts[0] == "00"
        && parts[1].len() == 32
        && parts[2].len() == 16
        && parts[3].len() == 2
        && parts[1..]
            .iter()
            .all(|p| p.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        && !value.contains("00000000000000000000000000000000-0000000000000000-00")
}

#[test]
fn wave47_l2t_traceparent_reaches_the_wire_inside_active_turn() {
    let (url, rx) = spawn_raw_capture_server();

    // 活跃 turn：收口点的注入前提。
    let _guard = begin_turn_trace("wave47-session", "wave47-turn", 1);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .expect("构建 reqwest client 失败");
    let request = with_current_traceparent(client.get(&url));

    let status = block_on(async move {
        let response = request.send().await.expect("出网请求失败");
        response.status().as_u16()
    });
    assert_eq!(status, 200, "本地探针服务应返回 200");

    let raw = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("未捕获到线上请求（服务未收到连接）");

    eprintln!("[WIRE-RECEIPT] 线上原始请求头：\n{raw}");
    let traceparent = extract_traceparent(&raw)
        .unwrap_or_else(|| panic!("线上请求缺少 traceparent 头，原始报文：\n{raw}"));
    assert!(
        is_w3c_traceparent(&traceparent),
        "traceparent 不符合 W3C 形态：{traceparent}"
    );
}

#[test]
fn wave47_l2at_no_traceparent_on_wire_without_active_turn() {
    let (url, rx) = spawn_raw_capture_server();

    // 反相对照：无活跃 turn，收口点必须原样透传，不得凭空写入头。
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
        .expect("构建 reqwest client 失败");
    let request = with_current_traceparent(client.get(&url));

    let status = block_on(async move {
        let response = request.send().await.expect("出网请求失败");
        response.status().as_u16()
    });
    assert_eq!(status, 200, "本地探针服务应返回 200");

    let raw = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("未捕获到线上请求（服务未收到连接）");

    assert!(
        extract_traceparent(&raw).is_none(),
        "无活跃 turn 时不得注入 traceparent，原始报文：\n{raw}"
    );
}
