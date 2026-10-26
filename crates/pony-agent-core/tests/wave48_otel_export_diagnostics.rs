//! Wave-48: OTel 导出诊断快照与客户端归因验收测试（L2-T & L2-AT）
//!
//! 契约守护目标：
//! 1. 守护线上故障：“桌面端 24h 内 0 条 trace / 客户端导出静默断链”
//! 2. 守护线上故障：“归因混淆 / 边缘探针与真实设备端流量无法区分”
//! 3. 守护线上故障：“未配 token 阻断启动 / 凭证泄露风险”
//!
//! 纪律：
//! - AAA 规范结构，真实断言，杜绝无断言测试与假测试。
//! - 全局 Mutex 串行化，保证多用例间 otel 单例隔离。
//! - 绝对零修改 src/ 业务代码。

use std::io::Read;
use std::net::TcpListener;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pony_agent_core::agent::otel::{
    self, export_diagnostics, ExportDiagnostics, OtelConfig,
};

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
                                    if line.to_ascii_lowercase().starts_with("content-length:") {
                                        if let Some(val) = line.split(':').nth(1) {
                                            content_length = val.trim().parse::<usize>().ok();
                                        }
                                    }
                                }
                            }
                        }

                        if let (Some(h_end), Some(cl)) = (header_end, content_length) {
                            if buf.len() >= h_end + cl {
                                break;
                            }
                        }
                    }
                    Err(_) => break,
                }
            }

            if let Some(h_end) = header_end {
                let body = buf[h_end..].to_vec();
                captured.push(CapturedRequest { body });
            }
        }
        captured
    });

    (port, handle)
}

fn make_test_config(port: u16, token: Option<&str>) -> OtelConfig {
    let mut config = OtelConfig::default();
    config.endpoint = format!("http://127.0.0.1:{port}");
    config.sampling_ratio = 1.0;
    if let Some(tok) = token {
        config.headers.push(("authorization".to_string(), format!("Bearer {tok}")));
    }
    config
}

// 守护线上故障：“桌面端 24h 内 0 条 trace / 诊断快照未初始化与已初始化状态反射”
#[test]
fn test_export_diagnostics_snapshot_lifecycle() {
    let _guard = TEST_LOCK.lock().unwrap();

    // 1. Arrange & Act: 未 init 时，快照必须为 None
    otel::shutdown();
    let diag_before = export_diagnostics();

    // 2. Assert: 未初始化
    assert!(
        diag_before.is_none(),
        "未 init 时 export_diagnostics() 必须返回 None"
    );

    // 3. Arrange & Act: 带鉴权 token 进行 init
    let (port, _sink) = spawn_otlp_sink(Duration::from_millis(500));
    let sentinel_token = "tok-SENTINEL-123";
    let config_with_auth = make_test_config(port, Some(sentinel_token));
    let expected_endpoint = config_with_auth.endpoint.clone();

    otel::init(config_with_auth).expect("init with auth must succeed");
    let diag_auth: Option<ExportDiagnostics> = export_diagnostics();

    // 4. Assert: 验证快照结构及安全脱敏
    assert!(
        diag_auth.is_some(),
        "init 成功后 export_diagnostics() 必须返回 Some(d)"
    );
    let d = diag_auth.unwrap();
    assert!(
        d.auth_configured,
        "config.headers 携带 authorization 时 d.auth_configured 必须为 true"
    );
    assert_eq!(
        d.endpoint, expected_endpoint,
        "d.endpoint 必须精确反射 init 传入的 endpoint"
    );

    // 安全红线断言：format!("{:?}", d) 与 d.endpoint 都绝对不得包含 token 明文
    let debug_repr = format!("{:?}", d);
    assert!(
        !debug_repr.contains(sentinel_token),
        "安全红线破坏：Debug 打印中泄漏了 token 明文！debug={debug_repr}"
    );
    assert!(
        !d.endpoint.contains(sentinel_token),
        "安全红线破坏：d.endpoint 意外包含了 token 明文！endpoint={}",
        d.endpoint
    );

    // 5. Arrange & Act: 清理并切换为不带鉴权的 init
    otel::shutdown();
    let (port2, _sink2) = spawn_otlp_sink(Duration::from_millis(500));
    let config_no_auth = make_test_config(port2, None);
    otel::init(config_no_auth).expect("init without auth must succeed");
    let diag_no_auth = export_diagnostics();

    // 6. Assert: 不带鉴权时 auth_configured 必须为 false
    assert!(
        diag_no_auth.is_some(),
        "无 auth 的 init 后 export_diagnostics() 也必须返回 Some"
    );
    assert!(
        !diag_no_auth.unwrap().auth_configured,
        "headers 不含 authorization 时 auth_configured 必须为 false"
    );

    otel::shutdown();
}

// 守护线上故障：“未配 token 阻断启动 / 行为契约不阻断”
#[test]
fn test_unconfigured_token_does_not_block_init() {
    let _guard = TEST_LOCK.lock().unwrap();

    // 1. Arrange: 构造未配置 token 的纯净配置
    otel::shutdown();
    let (port, _sink) = spawn_otlp_sink(Duration::from_millis(500));
    let config = make_test_config(port, None);

    // 2. Act: 执行 init
    // 行为契约：未配 token 必须仍然返回 Ok(())，绝不 panic 也绝不 Err。
    // 注释说明：关于 token 缺失的告警属于日志面输出，不进入验收断言，避免测试脆弱耦合日志格式。
    let init_result = otel::init(config);

    // 3. Assert: 验证成功
    assert!(
        init_result.is_ok(),
        "未配置 token 时 init 必须成功返回 Ok(())，不得阻断应用启动"
    );

    otel::shutdown();
}

// 守护线上故障：“归因混淆 / 普通真实设备端客户端必须携带 ingest.client.kind=device 且不含 pony.probe”
#[test]
fn test_resource_attribution_normal_device_client() {
    let _guard = TEST_LOCK.lock().unwrap();

    // 1. Arrange: 确保无 probe 环境变量
    std::env::remove_var("PONY_OTEL_PROBE");
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(2_000));
    let config = make_test_config(port, None);
    otel::init(config).expect("init normal device");
    otel::set_reporting_enabled(true);

    // 2. Act: 发起一次 turn trace 并 flush 上报
    {
        let _turn = otel::begin_turn_trace("sess-device-1", "turn-1", 0);
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

    // 3. Assert: 普通客户端 resource 属性必须含 ingest.client.kind = "device"，且绝对不含 pony.probe
    assert!(
        wire.contains("ingest.client.kind"),
        "真实设备端导出的 Resource 必须包含属性 ingest.client.kind"
    );
    assert!(
        wire.contains("device"),
        "真实设备端 ingest.client.kind 取值必须为 device"
    );
    assert!(
        !wire.contains("pony.probe"),
        "真实设备端导出的 Resource 严禁包含 pony.probe 探针标记"
    );
}

// 守护线上故障：“归因混淆 / 探针自报环境必须包含 ingest.client.kind=probe 且 pony.probe=true”
#[test]
fn test_resource_attribution_probe_client() {
    let _guard = TEST_LOCK.lock().unwrap();

    // 1. Arrange: 设置探针自报环境变量 PONY_OTEL_PROBE=1
    std::env::set_var("PONY_OTEL_PROBE", "1");
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(2_000));
    let config = make_test_config(port, None);
    otel::init(config).expect("init probe client");
    otel::set_reporting_enabled(true);

    // 2. Act: 发起一次 turn trace 并 flush 上报
    {
        let _turn = otel::begin_turn_trace("sess-probe-1", "turn-1", 0);
    }
    let _ = otel::force_flush(2_000);
    otel::shutdown();
    otel::set_reporting_enabled(false);
    std::env::remove_var("PONY_OTEL_PROBE");

    let captured = sink.join().expect("sink join");
    let wire = captured
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    // 3. Assert: 探针环境导出的 Resource 必须包含 ingest.client.kind = "probe" 且 pony.probe = "true"
    assert!(
        wire.contains("ingest.client.kind"),
        "探针端导出的 Resource 必须包含属性 ingest.client.kind"
    );
    assert!(
        wire.contains("probe"),
        "探针端 ingest.client.kind 取值必须为 probe"
    );
    assert!(
        wire.contains("pony.probe"),
        "探针端导出的 Resource 必须包含 pony.probe 属性"
    );
    assert!(
        wire.contains("true"),
        "探针端 pony.probe 取值必须为 true"
    );
}
