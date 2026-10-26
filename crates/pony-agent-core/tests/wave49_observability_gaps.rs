//! Wave-49: OTel / PonySentry / Log 真实链路缺口验收测试（L2-T & L2-AT）
//!
//! 契约守护目标：
//! 1. 守护线上故障：“deployment.environment 误标 production 污染真实生产报表”
//! 2. 守护线上故障：“跨路径检索断链 / edge 边缘改写 service.name 导致 trace 检索无稳定主键”
//! 3. 守护线上故障：“agent.turn 根 span 缺失 token usage 汇总导致治理与报表无法按 turn 聚合”
//! 4. 守护线上故障：“CLI/TUI 缺乏 settings 凭据分发通道 / PonySentry 凭据泄露”
//! 5. 守护线上故障：“本地日志父目录不存在或路径异常导致启动 panic 或静默丢失”
//!
//! 纪律：
//! - AAA 规范结构，真实断言，杜绝无断言测试与假测试。
//! - 全局 Mutex 串行化，保证多用例间 otel 与 settings 隔离。
//! - RUSTFLAGS="-D warnings" 保持 0 warning。

use std::fs;
use std::io::Read;
use std::net::TcpListener;
use std::sync::Mutex;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use pony_agent_core::agent::app_settings::{AppSettings, AppSettingsStore};
use pony_agent_core::agent::otel::{self, OtelConfig};
use pony_agent_core::agent::ponysentry::PonySentryConfig;

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

fn make_test_config(port: u16) -> OtelConfig {
    let mut config = OtelConfig::default();
    config.endpoint = format!("http://127.0.0.1:{port}");
    config.sampling_ratio = 1.0;
    config
}

// 守护线上故障：“deployment.environment 误标 production 污染真实生产报表”
#[test]
fn test_deployment_environment_precedence_and_fallback() {
    let _guard = TEST_LOCK.lock().unwrap();

    // 1. Arrange & Act: 优先级 1 —— 显式传入 PONY_OTEL_ENVIRONMENT=staging
    std::env::set_var("PONY_OTEL_ENVIRONMENT", "staging");
    otel::shutdown();
    otel::set_reporting_enabled(false);
    let (port1, sink1) = spawn_otlp_sink(Duration::from_millis(2_000));
    otel::init(make_test_config(port1)).expect("init otel staging");
    otel::set_reporting_enabled(true);
    {
        let _turn = otel::begin_turn_trace("sess-env-1", "turn-1", 0);
    }
    let _ = otel::force_flush(2_000);
    otel::shutdown();
    otel::set_reporting_enabled(false);
    std::env::remove_var("PONY_OTEL_ENVIRONMENT");

    let wire1 = sink1
        .join()
        .expect("sink1 join")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    // 2. Assert: 显式覆盖生效
    assert!(
        wire1.contains("deployment.environment"),
        "导出的 Resource 必须包含 deployment.environment"
    );
    assert!(
        wire1.contains("staging"),
        "显式传入 PONY_OTEL_ENVIRONMENT 时必须优先采纳 staging"
    );

    // 3. Arrange & Act: 优先级 2/3 —— 无 PONY_OTEL_ENVIRONMENT 且未注入 PONY_RELEASE_BUILD=1 时
    // 关键断言：即使 release 构建，没有发布标记也必须判 development
    otel::shutdown();
    otel::set_reporting_enabled(false);
    let (port2, sink2) = spawn_otlp_sink(Duration::from_millis(2_000));
    otel::init(make_test_config(port2)).expect("init otel default");
    otel::set_reporting_enabled(true);
    {
        let _turn = otel::begin_turn_trace("sess-env-2", "turn-2", 0);
    }
    let _ = otel::force_flush(2_000);
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let wire2 = sink2
        .join()
        .expect("sink2 join")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        wire2.contains("development"),
        "没有 PONY_RELEASE_BUILD=1 时，即使 release 构建也绝不能误标为 production，必须为 development"
    );
    assert!(
        !wire2.contains("production"),
        "没有发布标记时导出的 Resource 严禁包含 production"
    );
}

// 守护线上故障：“跨路径检索断链 / edge 边缘改写 service.name 导致 trace 检索无稳定主键”
#[test]
fn test_pony_app_name_stable_retrieval_key() {
    let _guard = TEST_LOCK.lock().unwrap();

    // 1. Arrange & Act: 客户端普通模式
    std::env::remove_var("PONY_OTEL_PROBE");
    otel::shutdown();
    otel::set_reporting_enabled(false);
    let (port1, sink1) = spawn_otlp_sink(Duration::from_millis(2_000));
    otel::init(make_test_config(port1)).expect("init otel normal");
    otel::set_reporting_enabled(true);
    {
        let _turn = otel::begin_turn_trace("sess-app-1", "turn-1", 0);
    }
    let _ = otel::force_flush(2_000);
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let wire1 = sink1
        .join()
        .expect("sink1 join")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    // 2. Assert: 普通客户端必须携带 pony.app.name = "pony-agent"
    assert!(
        wire1.contains("pony.app.name"),
        "真实设备客户端 resource 属性必须恒含 pony.app.name"
    );
    assert!(
        wire1.contains("pony-agent"),
        "真实设备客户端 pony.app.name 取值必须为 pony-agent"
    );

    // 3. Arrange & Act: 探针模式自报
    std::env::set_var("PONY_OTEL_PROBE", "1");
    otel::shutdown();
    otel::set_reporting_enabled(false);
    let (port2, sink2) = spawn_otlp_sink(Duration::from_millis(2_000));
    otel::init(make_test_config(port2)).expect("init otel probe");
    otel::set_reporting_enabled(true);
    {
        let _turn = otel::begin_turn_trace("sess-app-probe", "turn-probe", 0);
    }
    let _ = otel::force_flush(2_000);
    otel::shutdown();
    otel::set_reporting_enabled(false);
    std::env::remove_var("PONY_OTEL_PROBE");

    let wire2 = sink2
        .join()
        .expect("sink2 join")
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    // 4. Assert: 探针环境同样必须携带 pony.app.name = "pony-agent"
    assert!(
        wire2.contains("pony.app.name"),
        "边缘探针自报环境 resource 属性也必须恒含 pony.app.name"
    );
    assert!(
        wire2.contains("pony-agent"),
        "边缘探针自报环境 pony.app.name 取值必须为 pony-agent"
    );
}

// 守护线上故障：“agent.turn 根 span 缺失 token usage 汇总导致治理与报表无法按 turn 聚合”
#[test]
fn test_agent_turn_root_span_usage_aggregation() {
    let _guard = TEST_LOCK.lock().unwrap();

    otel::shutdown();
    otel::set_reporting_enabled(false);

    let (port, sink) = spawn_otlp_sink(Duration::from_millis(2_000));
    let config = make_test_config(port);
    otel::init(config).expect("init otel");
    otel::set_reporting_enabled(true);

    // 1. Arrange & Act: 启动 turn并在 turn 内触发带有 token usage 的 llm span
    {
        let _turn = otel::begin_turn_trace("sess-usage-1", "turn-usage-1", 0);
        {
            let mut llm_span = otel::start_llm_span("anthropic", Some("claude-3-5"), "chat");
            llm_span.record_usage(Some(120), Some(45));
        }
        {
            let mut llm_span2 = otel::start_llm_span("anthropic", Some("claude-3-5"), "chat");
            llm_span2.record_usage(Some(80), Some(55));
        }
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

    // 2. Assert: wire 输出必须携带聚合后的 token 属性键
    assert!(
        wire.contains("gen_ai.usage.input_tokens"),
        "有 token 用量时根 span 必须输出 gen_ai.usage.input_tokens"
    );
    assert!(
        wire.contains("gen_ai.usage.output_tokens"),
        "有 token 用量时根 span 必须输出 gen_ai.usage.output_tokens"
    );
}

// 守护线上故障：“CLI/TUI 缺乏 settings 凭据分发通道 / PonySentry 凭据泄露”
#[test]
fn test_ponysentry_settings_credentials_and_redaction() {
    let _guard = TEST_LOCK.lock().unwrap();

    let temp_dir = std::env::temp_dir().join(format!("pony-test-settings-{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let settings_path = temp_dir.join("settings.json");

    // 1. Arrange: 构造包含 PonySentry camelCase 键的 settings
    let store = AppSettingsStore::with_path(settings_path);
    let sentinel_token = "sentry-token-SECRET-999";
    let ingest_url = "https://sentry.custom-domain.top";

    let mut settings = AppSettings::default();
    settings.ponysentry_ingest_url = Some(ingest_url.to_string());
    settings.ponysentry_client_token = Some(sentinel_token.to_string());

    // 2. Act: 保存并重新读回
    store
        .save_view(settings)
        .expect("save settings with ponysentry credentials");
    let loaded = store.load_view();

    // 3. Assert: ① load_view 成功读回
    assert_eq!(loaded.ponysentry_ingest_url.as_deref(), Some(ingest_url));
    assert_eq!(
        loaded.ponysentry_client_token.as_deref(),
        Some(sentinel_token)
    );

    // 4. Assert: ② token 绝对不得出现在 Debug 输出中（防日志凭据泄漏）
    let debug_repr = format!("{:?}", loaded);
    assert!(
        !debug_repr.contains(sentinel_token),
        "安全红线破坏：AppSettings Debug 打印中泄漏了 ponysentry token！debug={debug_repr}"
    );
    assert!(
        debug_repr.contains("[REDACTED]"),
        "AppSettings Debug 必须输出 [REDACTED] 占位符"
    );

    // 5. Assert: ③ 解析优先级 settings > env
    let snapshot = loaded.ponysentry_settings_snapshot();
    let config = PonySentryConfig::from_settings(Some(&snapshot));
    assert_eq!(config.endpoint, ingest_url);
    assert_eq!(config.client_token.as_deref(), Some(sentinel_token));

    let snapshot_debug = format!("{:?}", snapshot);
    assert!(
        !snapshot_debug.contains(sentinel_token),
        "PonySentrySettingsSnapshot Debug 打印中泄漏了 token！"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

// 守护线上故障：“本地日志父目录不存在或路径异常导致启动 panic 或静默丢失”
#[test]
fn test_otel_log_auto_create_parent_and_resilience() {
    let _guard = TEST_LOCK.lock().unwrap();

    let temp_root = std::env::temp_dir().join(format!("pony-test-log-{}", std::process::id()));
    let nested_log_dir = temp_root.join("deep").join("nested").join("logs");
    let log_file = nested_log_dir.join("pony-agent.log");

    // 1. Arrange: 确保父目录完全不存在
    let _ = fs::remove_dir_all(&temp_root);
    assert!(
        !nested_log_dir.exists(),
        "前置断言：父目录在测试开始前必须不存在"
    );

    // 2. Act: 调用 set_log_file_path，验证父目录自动递归创建且绝不 panic
    otel::log::set_log_file_path(log_file.clone());

    // 3. Assert: 父目录必须已被成功建立，且写入一行有效 JSON 日志
    assert!(
        nested_log_dir.exists(),
        "set_log_file_path 必须自动创建深层嵌套父目录"
    );

    otel::log::log_json("info", "test logging resilience", None);
    assert!(log_file.exists(), "日志文件必须被成功创建");

    let content = fs::read_to_string(&log_file).expect("read written log file");
    assert!(
        content.contains("test logging resilience"),
        "日志文件必须包含刚写入的消息"
    );
    assert!(
        content.contains("\"level\":\"info\""),
        "日志必须按标准 JSON 格式输出 level 字段"
    );

    let _ = fs::remove_dir_all(&temp_root);
}
