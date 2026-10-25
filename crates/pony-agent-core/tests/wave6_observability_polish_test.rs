//! Wave-6 可观测性精细化补齐与 Issue↔Trace 行为级红相验收测试 (L2-T / L2-AT)
//!
//! 契约来源：.dev-team/contracts/wave-6-observability-polish.md
//! 覆盖维度：
//! 1. K1.1 计价协议语义归一（Anthropic vs OpenAI 对 (input=100, cache_read=5000, output=0) 的断言）
//! 2. K4.1 子 Span 会话属性下沉（活跃 turn 时 llm.chat 与 tool.call 的 attributes 必须含 langfuse.session.id，空闲时不得出现）
//! 3. K6 结构化日志数值字段扩展（成本 warn 日志必须包含数值型的 turn_cost_usd 与 budget_usd）
//! 4. 真实运行暴露的 BLOCKING：已注册活跃 turn 周期内，ponysentry::capture_turn_failure 上报 payload 的 trace_id 必须等于 exported_trace_id()

use pony_agent_core::agent::cost::{self, ModelPricing, ProviderCostProtocol};
use pony_agent_core::agent::otel::{self, ToolSpanInput};
use pony_agent_core::agent::ponysentry::{self, PonySentryConfig};
use std::sync::Mutex;
use std::time::Duration;

static LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// 1. K1.1 计价归一：Anthropic vs OpenAI 语义
// ---------------------------------------------------------------------------
#[test]
fn k1_1_cost_protocol_semantics_anthropic_vs_openai() {
    let pricing = ModelPricing {
        input_per_mtok: 3.0,
        output_per_mtok: 15.0,
        cache_read_per_mtok: 0.3,
    };

    let input = 100u64;
    let cache_read = 5000u64;
    let output = 0u64;

    // 1. Anthropic 语义：input_tokens 不含 cache_read，因此未缓存部分严格为 input = 100
    // uncached_input_cost = 100 * 3.0 / 1e6 = 0.000300
    // cache_cost = 5000 * 0.3 / 1e6 = 0.001500
    // total = 0.001800
    let anthropic_est = cost::calculate_cost_with_protocol(
        &pricing,
        input,
        output,
        Some(cache_read),
        ProviderCostProtocol::Anthropic,
    )
    .expect("Anthropic 计费必须成功");

    assert_eq!(
        anthropic_est.input_cost_usd,
        0.0018,
        "Anthropic 语义下未命中文档输入不能被减为 0（现状会减为 0，预期红相）"
    );

    // 2. OpenAI 语义：prompt_tokens 已包含 cached_tokens，未缓存部分为 max(100 - 5000, 0) = 0
    // uncached_input_cost = 0
    // cache_cost = 5000 * 0.3 / 1e6 = 0.001500
    // total = 0.001500
    let openai_est = cost::calculate_cost_with_protocol(
        &pricing,
        input,
        output,
        Some(cache_read),
        ProviderCostProtocol::OpenAi,
    )
    .expect("OpenAI 计费必须成功");

    assert_eq!(
        openai_est.input_cost_usd,
        0.0015,
        "OpenAI 语义下未命中文档输入应当为 0"
    );

    // 3. 未知协议：按 OpenAI 语义保守处理
    let unknown_est = cost::calculate_cost_with_protocol(
        &pricing,
        input,
        output,
        Some(cache_read),
        ProviderCostProtocol::Unknown,
    )
    .expect("未知协议计费必须成功");

    assert_eq!(
        unknown_est.input_cost_usd,
        openai_est.input_cost_usd,
        "未知协议必须保守回退到 OpenAI 语义"
    );
}

// ---------------------------------------------------------------------------
// 2. K4.1 子 Span 会话属性 langfuse.session.id 下沉
// ---------------------------------------------------------------------------
#[test]
fn k4_1_child_spans_contain_session_id_on_wire() {
    let _g = LOCK.lock().unwrap();
    otel::shutdown();

    let (port, sink) = test_helpers::spawn_otlp_sink(Duration::from_millis(2000));
    otel::init(test_helpers::local_config(port)).expect("本地端点装配必须成功");
    otel::set_reporting_enabled(true);

    let session_id = "sess-wave6-k41";

    // 1. 有活跃 turn 时：创建 llm.chat 与 tool.call 子 span
    {
        let _turn = otel::begin_turn_trace(session_id, "turn-k41", 1);

        let mut llm_span = otel::start_llm_span("openai", Some("gpt-4o"), "chat");
        llm_span.record_usage(Some(100), Some(50));
        drop(llm_span);

        otel::record_tool_call_span(&ToolSpanInput {
            tool_name: "bash".to_string(),
            status: "success".to_string(),
            duration_ms: Some(25),
            error: None,
        });
    }

    let flushed = otel::force_flush(3000);
    assert!(flushed.is_ok());

    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured = sink.join().expect("sink 线程不得 panic");
    assert!(!captured.is_empty(), "必须收到 OTLP 导出请求");

    let wire_text = captured
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    // 断言 wire body 中 llm.chat 与 tool.call 的 attributes 区域存在 langfuse.session.id
    assert!(
        wire_text.contains("langfuse.session.id"),
        "Wire payload 必须包含 langfuse.session.id 属性（现状子 span 未带，必须红）"
    );
    assert!(
        wire_text.contains(session_id),
        "Wire payload 必须包含 session_id 取值"
    );
}

// ---------------------------------------------------------------------------
// 3. K6 结构化日志数值字段扩展
// ---------------------------------------------------------------------------
#[test]
fn k6_cost_budget_warning_log_contains_numerical_fields() {
    let _g = LOCK.lock().unwrap();

    // 捕获日志输出测试：log_json_with_fields 输出结构化 extras
    let mut log_output = Vec::new();
    otel::log::log_json_with_extras(
        "warn",
        "turn cost reached or exceeded budget threshold",
        Some("turn-cost-log-1"),
        &[
            ("turn_id", serde_json::json!("turn-1234")),
            ("turn_cost_usd", serde_json::json!(0.0075)),
            ("budget_usd", serde_json::json!(0.000001)),
        ],
        &mut log_output,
    );

    let log_str = String::from_utf8(log_output).expect("日志输出必须为合法 UTF-8");
    let json_val: serde_json::Value = serde_json::from_str(&log_str).expect("必须是合法 JSON 行");

    assert_eq!(json_val["level"], "warn");
    assert_eq!(json_val["message"], "turn cost reached or exceeded budget threshold");
    // 数值型校验：必须是 number 类型，绝不能是 string
    assert!(json_val["turn_cost_usd"].is_f64() || json_val["turn_cost_usd"].is_number());
    assert_eq!(json_val["turn_cost_usd"], 0.0075);
    assert!(json_val["budget_usd"].is_f64() || json_val["budget_usd"].is_number());
    assert_eq!(json_val["budget_usd"], 0.000001);
}

// ---------------------------------------------------------------------------
// 4. 真实运行暴露的 BLOCKING：capture_turn_failure 必须在活跃 turn 内获得真实 trace_id
// ---------------------------------------------------------------------------
#[test]
fn blocking_capture_turn_failure_retains_exported_trace_id() {
    let _g = LOCK.lock().unwrap();

    // 初始化 PonySentry 客户端（设置 dummy token 使 capture_error 能进入上报/捕获逻辑）
    ponysentry::init(PonySentryConfig {
        client_token: Some("dummy-token".to_string()),
        enabled: true,
        endpoint: "http://127.0.0.1:9".to_string(),
        environment: "test".to_string(),
        release: "test".to_string(),
    });
    ponysentry::enable_turn_failure_capture(true);

    otel::shutdown();
    let (port, _sink) = test_helpers::spawn_otlp_sink(Duration::from_millis(1000));
    otel::init(test_helpers::local_config(port)).expect("本地端点装配必须成功");
    otel::set_reporting_enabled(true);

    let turn_id = "turn-blocking-test";
    let session_id = "sess-blocking-test";

    // 开启真实活跃 turn
    let _turn_scope = otel::begin_turn_trace(session_id, turn_id, 1);
    let expected_trace_id = otel::exported_trace_id();
    assert!(
        expected_trace_id.is_some(),
        "活跃 turn 必须具有导出的 trace_id"
    );

    // 调用 turn_flow 失败时触发的旁路上报函数
    ponysentry::capture_turn_failure(turn_id, "API call failed with 500", Some("claude-3-7-sonnet"));

    // 检查最后入队的 payload trace_id
    let last_payload = ponysentry::testing_last_enqueued_payload();
    assert!(
        last_payload.is_some(),
        "PonySentry 必须成功入队 failure payload"
    );
    let p = last_payload.unwrap();

    assert_eq!(
        p.trace_id,
        expected_trace_id,
        "BLOCKING 缺陷断言：capture_turn_failure 上报的 trace_id 必须与活跃 turn 的 exported_trace_id 相等"
    );
}

mod test_helpers {
    use super::*;
    use pony_agent_core::agent::otel::OtelConfig;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    pub struct CapturedRequest {
        pub body: Vec<u8>,
    }

    pub fn local_config(port: u16) -> OtelConfig {
        OtelConfig {
            endpoint: format!("http://127.0.0.1:{port}"),
            service_name: "test-wave6".to_string(),
            sampling_ratio: 1.0,
            headers: vec![],
        }
    }

    pub fn spawn_otlp_sink(
        timeout: Duration,
    ) -> (u16, thread::JoinHandle<Vec<CapturedRequest>>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("绑定端口");
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();

        let handle = thread::spawn(move || {
            let start = std::time::Instant::now();
            let mut captured = Vec::new();

            while start.elapsed() < timeout {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let mut chunk = [0u8; 65536];
                        let mut buf = Vec::new();
                        if let Ok(read) = stream.read(&mut chunk) {
                            buf.extend_from_slice(&chunk[..read]);
                        }
                        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/x-protobuf\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                        let _ = stream.flush();
                        captured.push(CapturedRequest { body: buf });
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
            captured
        });

        (port, handle)
    }
}
