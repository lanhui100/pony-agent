//! Wave-5 会话成本估算与预算软告警双模态红相测试 (L2-T / L2-AT)
//!
//! 契约来源：.dev-team/contracts/wave-5-cost.md (K1~K5)
//! 覆盖目标：
//! 1. K1 正例：已知模型阶梯单价（未缓存/缓存读取/输出），计算精确至 6 位小数
//! 2. K1 cache 不双重计价：未缓存输入严格为 max(input - cache, 0)，反例对比
//! 3. K1 未知模型返回 None，禁止任何 fallback/猜测
//! 4. K1 非有限/负数 token 返回 None
//! 5. K1 极大值（1e12）浮点稳定不溢出
//! 6. K2 TurnStreamEvent 包含 costUsd 字段，None 时序列化缺席
//! 7. K4 OTel 属性 wire 级断言：gen_ai.cost.usd 与 agent.turn.cost.usd 正确上报，门禁关闭时不出现
//! 8. K3 Settings 预算语义：cost_budget_usd <= 0 或 None 视为未设置预算

use pony_agent_core::agent::cost::{self, ModelPricing};
use pony_agent_core::agent::otel;
use pony_agent_core::agent::runtime::TurnStreamEvent;
use std::sync::Mutex;
use std::time::Duration;

static LOCK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// 1. K1 正例与单价计算契约
// ---------------------------------------------------------------------------
#[test]
fn k1_cost_calculation_exact_formula_and_precision() {
    // 构造测试价格：input 3.0, output 15.0, cache 0.75 USD per 1M tokens
    let pricing = ModelPricing {
        input_per_mtok: 3.0,
        output_per_mtok: 15.0,
        cache_read_per_mtok: 0.75,
    };

    // usage: input 1000, cache_read 400, output 500
    // uncached_input = 1000 - 400 = 600
    // input_cost = (600 * 3.0 + 400 * 0.75) / 1e6 = (1800 + 300) / 1e6 = 2100 / 1e6 = 0.002100
    // output_cost = (500 * 15.0) / 1e6 = 7500 / 1e6 = 0.007500
    // total_cost = 0.002100 + 0.007500 = 0.009600
    let estimate = cost::calculate_cost_with_pricing(&pricing, 1000, 500, Some(400))
        .expect("计价必须成功");

    assert_eq!(estimate.input_cost_usd, 0.0021);
    assert_eq!(estimate.output_cost_usd, 0.0075);
    assert_eq!(estimate.total_cost_usd, 0.0096);
}

// ---------------------------------------------------------------------------
// 2. K1 Cache 折扣不双重计费断言
// ---------------------------------------------------------------------------
#[test]
fn k1_cache_tokens_not_double_charged() {
    let pricing = ModelPricing {
        input_per_mtok: 10.0,
        output_per_mtok: 30.0,
        cache_read_per_mtok: 1.0,
    };

    let input = 10_000;
    let cache_read = 6_000;
    let output = 1_000;

    let estimate = cost::calculate_cost_with_pricing(&pricing, input, output, Some(cache_read))
        .expect("计价成功");

    // 严禁双重计费反例断言：
    // 若按 input 全额计费：(10000 * 10 + 6000 * 1) / 1e6 = 0.106
    // 契约正确计费：((10000 - 6000) * 10 + 6000 * 1) / 1e6 = (40000 + 6000) / 1e6 = 0.046
    let wrong_double_charged_input_cost = (input as f64 * pricing.input_per_mtok + cache_read as f64 * pricing.cache_read_per_mtok) / 1_000_000.0;
    assert_ne!(
        estimate.input_cost_usd,
        wrong_double_charged_input_cost,
        "严禁将 cache token 在 input 中重复计价"
    );
    assert_eq!(estimate.input_cost_usd, 0.046);
}

// ---------------------------------------------------------------------------
// 3. K1 未知模型返回 None 契约
// ---------------------------------------------------------------------------
#[test]
fn k1_unknown_model_must_return_none() {
    let unknown_model = "my-custom-fine-tuned-model-xyz";
    let res = cost::estimate_cost(unknown_model, 1000, 500, None);
    assert!(
        res.is_none(),
        "未知模型必须严格返回 None，禁止回退默认或猜测价"
    );
}

// ---------------------------------------------------------------------------
// 4. K1 非有限与负数防御
// ---------------------------------------------------------------------------
#[test]
fn k1_invalid_negative_or_nan_returns_none() {
    // 负数单价或非法输入
    let invalid_pricing = ModelPricing {
        input_per_mtok: -2.0,
        output_per_mtok: 8.0,
        cache_read_per_mtok: 0.5,
    };
    assert!(cost::calculate_cost_with_pricing(&invalid_pricing, 1000, 500, None).is_none());

    let nan_pricing = ModelPricing {
        input_per_mtok: f64::NAN,
        output_per_mtok: 8.0,
        cache_read_per_mtok: 0.5,
    };
    assert!(cost::calculate_cost_with_pricing(&nan_pricing, 1000, 500, None).is_none());
}

// ---------------------------------------------------------------------------
// 5. K1 极大值不溢出
// ---------------------------------------------------------------------------
#[test]
fn k1_astronomical_token_numbers_do_not_overflow() {
    let pricing = ModelPricing {
        input_per_mtok: 5.0,
        output_per_mtok: 20.0,
        cache_read_per_mtok: 1.0,
    };

    // 1万亿 token
    let huge_tokens = 1_000_000_000_000u64;
    let res = cost::calculate_cost_with_pricing(&pricing, huge_tokens, huge_tokens, Some(huge_tokens / 2));
    assert!(res.is_some());
    let est = res.unwrap();
    assert!(est.total_cost_usd.is_finite());
    assert!(est.total_cost_usd > 0.0);
}

// ---------------------------------------------------------------------------
// 6. K2 传输契约：costUsd 命名与 None 缺席
// ---------------------------------------------------------------------------
#[test]
fn k2_event_cost_usd_camel_case_and_skip_serializing_none() {
    let mut event = TurnStreamEvent {
        event_id: None,
        session_id: None,
        turn_id: "turn-cost-wire".to_string(),
        trace_id: None,
        kind: "completed".to_string(),
        event_type: None,
        event_version: None,
        sequence: None,
        emitted_at_ms: None,
        phase: Some("completed".to_string()),
        text: None,
        reasoning_content: None,
        error: None,
        provider_requested_name: None,
        provider_name: None,
        provider_protocol: None,
        provider_model: None,
        provider_source: None,
        provider_mode: None,
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: Some(1000),
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: Some(500),
        total_tokens: Some(1500),
        first_token_latency_ms: None,
        turn_duration_ms: None,
        trace_steps: None,
        trace_timeline: None,
        tool_activities: None,
        provider_call_records: None,
        hook_trace_records: None,
        session_summary: None,
        step: None,
        cost_usd: Some(0.012345),
    };

    let serialized = serde_json::to_string(&event).expect("序列化必须成功");
    assert!(
        serialized.contains(r#""costUsd":0.012345"#),
        "必须序列化为 camelCase `costUsd`，实际输出: {serialized}"
    );

    // 当为 None 时，键缺席
    event.cost_usd = None;
    let serialized_none = serde_json::to_string(&event).expect("序列化必须成功");
    assert!(
        !serialized_none.contains("costUsd"),
        "None 时键必须缺席，实际输出: {serialized_none}"
    );
}

// ---------------------------------------------------------------------------
// 7. K4 OTel 属性 wire 级断言
// ---------------------------------------------------------------------------
#[test]
fn k4_otel_spans_export_cost_attributes_to_wire() {
    let _g = LOCK.lock().unwrap();
    otel::shutdown();

    let (port, sink) = test_helpers::spawn_otlp_sink(Duration::from_millis(1500));
    otel::init(test_helpers::local_config(port)).expect("本地端点装配必须成功");
    otel::set_reporting_enabled(true);

    {
        let mut turn = otel::begin_turn_trace("sess-cost", "turn-cost", 1);
        turn.record_turn_cost(Some(0.045678));

        let mut span = otel::start_llm_span("anthropic", Some("claude-3-5-sonnet"), "chat");
        span.record_cost(Some(0.012345));
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

    assert!(
        wire_text.contains("gen_ai.cost.usd"),
        "Wire payload 必须包含 gen_ai.cost.usd 属性"
    );
    assert!(
        wire_text.contains("agent.turn.cost.usd"),
        "Wire payload 必须包含 agent.turn.cost.usd 属性"
    );
}

// ---------------------------------------------------------------------------
// 7b. K4 OTel 属性负向断言：未知模型不写 cost 属性，门禁关闭时不导出
// ---------------------------------------------------------------------------
#[test]
fn k4_otel_spans_do_not_export_cost_when_unknown_model_or_gate_closed() {
    let _g = LOCK.lock().unwrap();

    // 负向 1：OTel 门禁关闭时，即使记录了 cost 也绝不产生任何 wire 导出请求
    otel::shutdown();
    let (port1, sink1) = test_helpers::spawn_otlp_sink(Duration::from_millis(1000));
    otel::init(test_helpers::local_config(port1)).expect("本地端点装配必须成功");
    otel::set_reporting_enabled(false); // 门禁关闭

    {
        let mut turn = otel::begin_turn_trace("sess-closed", "turn-closed", 1);
        turn.record_turn_cost(Some(0.05));
        let mut span = otel::start_llm_span("openai", Some("gpt-4o"), "chat");
        span.record_cost(Some(0.02));
    }

    let _ = otel::force_flush(1000);
    otel::shutdown();
    let captured1 = sink1.join().expect("sink 线程不得 panic");
    assert!(
        captured1.is_empty(),
        "K4 负向断言：OTel 门禁关闭时不得输出任何 wire 请求"
    );

    // 负向 2：门禁开启，但模型未知且 cost 为 None 时，wire payload 绝对不得包含 gen_ai.cost.usd
    let (port2, sink2) = test_helpers::spawn_otlp_sink(Duration::from_millis(1500));
    otel::init(test_helpers::local_config(port2)).expect("本地端点装配必须成功");
    otel::set_reporting_enabled(true); // 门禁开启

    {
        let mut turn = otel::begin_turn_trace("sess-unknown", "turn-unknown", 1);
        turn.record_turn_cost(None); // 未知模型无成本

        let mut span = otel::start_llm_span("unknown-prov", Some("unknown-model-xyz"), "chat");
        span.record_cost(None); // 未知模型无成本
    }

    let flushed = otel::force_flush(3000);
    assert!(flushed.is_ok());
    otel::shutdown();
    otel::set_reporting_enabled(false);

    let captured2 = sink2.join().expect("sink 线程不得 panic");
    assert!(!captured2.is_empty(), "必须收到正常 trace 导出请求");

    let wire_text2 = captured2
        .iter()
        .map(|r| String::from_utf8_lossy(&r.body).to_string())
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        !wire_text2.contains("gen_ai.cost.usd"),
        "K4 负向断言：未知模型（cost 为 None）时 wire payload 绝对不得包含 gen_ai.cost.usd"
    );
    assert!(
        !wire_text2.contains("agent.turn.cost.usd"),
        "K4 负向断言：未知模型（cost 为 None）时 wire payload 绝对不得包含 agent.turn.cost.usd"
    );
}

// ---------------------------------------------------------------------------
// 8. K3 Settings 预算语义
// ---------------------------------------------------------------------------
#[test]
fn k3_negative_or_zero_budget_is_treated_as_unset() {
    use pony_agent_core::agent::cost::is_budget_exceeded;

    // <= 0.0 视为未设置预算，永不触发超限
    assert_eq!(is_budget_exceeded(100.0, Some(0.0)), false);
    assert_eq!(is_budget_exceeded(100.0, Some(-5.0)), false);
    assert_eq!(is_budget_exceeded(100.0, None), false);

    // 正常超限
    assert_eq!(is_budget_exceeded(0.51, Some(0.50)), true);
    assert_eq!(is_budget_exceeded(0.49, Some(0.50)), false);
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
            service_name: "test-cost".to_string(),
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
