//! Wave43 覆盖继承守卫：PonySentry **trace** 下线后，**issue/错误上报**（保留面）的韧性契约仍须成立。
//!
//! 背景（契约矩阵 §5 / D6）：`ponysentry_trace_test.rs` 的
//! `test_queue_overflow_and_avalanche_defense` 随 trace 能力下线被删除，但它验证的
//! 「有界队列 + 溢出丢弃 + 入队永不阻塞」语义属于**保留面**——`client.rs` 的
//! `QUEUE_CAPACITY` / `try_send` 分支仍在服务 issue 上报。删除 trace 时顺手废掉这条防线
//! 不会有编译错误，只会在线上表现为「PonySentry 不可达 ⇒ 主链路被拖死」，因此在此独立守卫。
//!
//! 脱敏（原 `test_zero_trust_redaction_for_trace_and_secrets`）由
//! `ponysentry_telemetry_test.rs` 的 3 条 sanitizer 用例继承；issue 上报出口由
//! `ponysentry_credential_gate.rs` 的本地 egress 探针继承——本文件只补**队列韧性**缺口。

use std::path::PathBuf;
use std::time::{Duration, Instant};

use pony_agent_core::agent::ponysentry::{PonySentryClient, PonySentryConfig};

/// 不可达端点：worker 线程的连接立即失败，不产生真实外网出口。
fn unreachable_config() -> PonySentryConfig {
    PonySentryConfig {
        enabled: true,
        endpoint: "http://127.0.0.1:9".to_string(),
        client_token: Some("dummy-issue-token".to_string()),
        environment: "test".to_string(),
        release: "0.1.0".to_string(),
    }
}

fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn issue_reporting_burst_over_bounded_queue_never_blocks() {
    // Arrange：带 token 的真实入队路径（无 token 时 `capture_error` 会 fail-closed 早退，
    // 那样 burst 也会「秒过」——必须有 token 才能证明非阻塞来自有界队列丢弃而非早退）。
    let client = PonySentryClient::new(unreachable_config());

    // Act：突发 2000 次（> QUEUE_CAPACITY 1024），worker 侧对不可达端点持续失败。
    let started = Instant::now();
    for index in 0..2_000 {
        client.capture_error("BurstError", &format!("burst {index}"), None);
    }
    let elapsed = started.elapsed();

    // Assert：溢出必须走「丢弃 + 保护运行时」，入队路径不得阻塞（删除前基线阈值为 500ms）。
    assert!(
        elapsed < Duration::from_millis(500),
        "issue 上报突发入队耗时 {elapsed:?}；溢出时必须非阻塞丢弃（有界队列 >1024 仍须立即返回）"
    );

    // 队列被灌满后仍不得 panic / 卡死。
    client.flush();
}

#[test]
fn issue_queue_stays_bounded_and_drops_on_overflow() {
    // 行为断言之外补静态锚点：证明溢出分支是「有界 + 丢弃」，而不是无界堆积或阻塞发送。
    let path = crate_root().join("src/agent/ponysentry/client.rs");
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("证据源必须可读（{}）：{err}", path.display()));

    assert!(
        source.contains("const QUEUE_CAPACITY: usize = 1024;"),
        "issue 事件队列必须保持有界（QUEUE_CAPACITY = 1024），否则 Senty 不可达会拖垮主链路"
    );
    assert!(
        source.contains("try_send("),
        "入队必须使用非阻塞 try_send（禁止 blocking send 阻塞调用方）"
    );
    assert!(
        source.contains("TrySendError::Full"),
        "队列满必须显式走溢出丢弃分支（静默阻塞或 panic 均违反防雪崩契约）"
    );
    assert!(
        source.contains("dropping event to protect runtime"),
        "溢出分支必须留下可诊断的丢弃日志（删除该日志等于失去线上可观测性）"
    );
}
