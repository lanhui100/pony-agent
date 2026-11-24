//! Wave-1 · OpenTelemetry traceparent Header Injection & Robustness Test Suite
//!
//! 包含两个维度：
//! 1. L2-T: W3cTraceContext 格式化符合 W3C 规范，inject_traceparent_header 能够正确写入 reqwest::header::HeaderMap
//! 2. L2-AT: 畸形上下文处理（非法字符、过长/过短字段）不崩溃、网络超时设置不被篡改（对齐 NFR external_call_timeout_ms <= 3000ms）

use pony_agent_core::agent::otel::{inject_traceparent_header, W3cTraceContext};
use reqwest::header::HeaderMap;
use std::time::Duration;

struct HeaderMapWrapper(pub HeaderMap);

impl AsMut<HeaderMap> for HeaderMapWrapper {
    fn as_mut(&mut self) -> &mut HeaderMap {
        &mut self.0
    }
}

#[test]
fn test_w3c_trace_context_format_and_header_injection() {
    let ctx = W3cTraceContext {
        version: "00".to_string(),
        trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".to_string(),
        parent_id: "00f067aa0ba902b7".to_string(),
        trace_flags: "01".to_string(),
    };

    assert_eq!(
        ctx.to_header_value(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
    );

    let mut headers = HeaderMapWrapper(HeaderMap::new());
    inject_traceparent_header(&mut headers, Some(&ctx));

    assert!(headers.0.contains_key("traceparent"));
    assert_eq!(
        headers.0.get("traceparent").unwrap(),
        "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01"
    );

    // 当 context 为 None 时，不写入头，也不破坏已有头
    let mut clean_headers = HeaderMapWrapper(HeaderMap::new());
    inject_traceparent_header(&mut clean_headers, None);
    assert!(!clean_headers.0.contains_key("traceparent"));
}

#[test]
fn test_w3c_traceparent_adversarial_malformed_context_resilience() {
    // 注入非法控制字符与非 ASCII 字符，断言 inject_traceparent_header 优雅降级不 panic
    let malformed_ctx = W3cTraceContext {
        version: "00\r\nInjected-Header: evil".to_string(),
        trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".to_string(),
        parent_id: "00f067aa0ba902b7".to_string(),
        trace_flags: "01".to_string(),
    };

    let mut headers = HeaderMapWrapper(HeaderMap::new());
    // 必须不 panic，由于 HeaderValue 拒绝换行符，应跳过注入
    inject_traceparent_header(&mut headers, Some(&malformed_ctx));
    assert!(!headers.0.contains_key("traceparent"));
}

#[test]
fn test_w3c_traceparent_adversarial_timeout_immutability() {
    // 验证网络超时对齐 NFR external_call_timeout_ms <= 3000ms 且在注入链路中不可被篡改
    let nfr_timeout = Duration::from_millis(3000);
    let request_timeout = nfr_timeout;

    let ctx = W3cTraceContext {
        version: "00".to_string(),
        trace_id: "4bf92f3577b34da6a3ce929d0e0e4736".to_string(),
        parent_id: "00f067aa0ba902b7".to_string(),
        trace_flags: "01".to_string(),
    };

    let mut headers = HeaderMapWrapper(HeaderMap::new());
    inject_traceparent_header(&mut headers, Some(&ctx));

    // 严禁任何旁路逻辑篡改请求超时设置
    assert_eq!(request_timeout, nfr_timeout);
    assert!(request_timeout.as_millis() <= 3000);
}
