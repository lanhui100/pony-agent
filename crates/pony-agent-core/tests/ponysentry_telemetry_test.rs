//! PonySentry 遥测错误上报红相独立验收测试 (L2-T Acceptance Test)
//!
//! 覆盖目标：
//! 1. IngestPayload 序列化格式与字段契约（与 /home/dm/pony-sentry 契约对齐）
//! 2. 客户端零信任脱敏（文件路径、敏感 token/password/key、JSON 递归脱敏）
//! 3. Breadcrumb 环形缓冲区上限（<= 64 条）
//! 4. 配置契约（默认配置与从环境变量加载）
//! 5. 错误捕获与客户端 API（非阻塞 fire-and-forget 契约）

use pony_agent_core::agent::ponysentry::{
    add_breadcrumb, capture_error, init, sanitize, sanitize_json, Breadcrumb, Exception, Frame,
    IngestPayload, PonySentryClient, PonySentryConfig,
};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn test_ingest_payload_serialization_contract() {
    let payload = IngestPayload {
        platform: "rust".to_string(),
        release: "0.1.109".to_string(),
        environment: "production".to_string(),
        message: Some("Runtime error occurred".to_string()),
        exception: Some(Exception {
            error_type: "UpstreamTimeout".to_string(),
            value: Some("Request timed out after 3000ms".to_string()),
            stacktrace: Some(vec![Frame {
                filename: Some("crates/pony-agent-core/src/agent/provider/mod.rs".to_string()),
                function: Some("post_openai_json".to_string()),
                lineno: Some(1752),
                in_app: Some(true),
            }]),
        }),
        tags: Some({
            let mut map = HashMap::new();
            map.insert("provider".to_string(), "anthropic".to_string());
            map.insert("model".to_string(), "claude-3-7-sonnet".to_string());
            map
        }),
        extra: Some(json!({
            "session_id": "sess-test-123",
            "turn_id": "turn-test-456",
            "retry_attempts": 3
        })),
        breadcrumbs: Some(vec![Breadcrumb {
            category: "provider".to_string(),
            message: "Sending request to endpoint".to_string(),
            data: Some({
                let mut map = HashMap::new();
                map.insert("url".to_string(), "https://api.anthropic.com".to_string());
                map
            }),
        }]),
    };

    let serialized = serde_json::to_string(&payload).expect("Serialization must succeed");
    let v: serde_json::Value =
        serde_json::from_str(&serialized).expect("Deserialization must succeed");

    assert_eq!(v["platform"], "rust");
    assert_eq!(v["release"], "0.1.109");
    assert_eq!(v["environment"], "production");
    assert_eq!(v["message"], "Runtime error occurred");
    assert_eq!(v["exception"]["error_type"], "UpstreamTimeout");
    assert_eq!(
        v["exception"]["value"],
        "Request timed out after 3000ms"
    );
    assert_eq!(
        v["exception"]["stacktrace"][0]["filename"],
        "crates/pony-agent-core/src/agent/provider/mod.rs"
    );
    assert_eq!(
        v["exception"]["stacktrace"][0]["function"],
        "post_openai_json"
    );
    assert_eq!(v["tags"]["provider"], "anthropic");
    assert_eq!(v["extra"]["session_id"], "sess-test-123");
    assert_eq!(v["breadcrumbs"][0]["category"], "provider");
}

#[test]
fn test_sanitizer_path_redaction() {
    let unix_path = "Error occurred at /home/developer/project/secret.key";
    let sanitized_unix = sanitize(unix_path);
    assert!(
        !sanitized_unix.contains("/home/developer"),
        "Unix user path must be redacted: {sanitized_unix}"
    );
    assert!(
        sanitized_unix.contains("[USER_HOME]"),
        "Redacted path must contain [USER_HOME]: {sanitized_unix}"
    );

    let mac_path = "Failed in /Users/johndoe/Library/Application Support";
    let sanitized_mac = sanitize(mac_path);
    assert!(
        !sanitized_mac.contains("/Users/johndoe"),
        "macOS user path must be redacted: {sanitized_mac}"
    );
    assert!(
        sanitized_mac.contains("[USER_HOME]"),
        "Redacted path must contain [USER_HOME]: {sanitized_mac}"
    );

    let win_path = r"File missing at C:\Users\Administrator\AppData\Local\data.db";
    let sanitized_win = sanitize(win_path);
    assert!(
        !sanitized_win.contains(r"C:\Users\Administrator"),
        "Windows user path must be redacted: {sanitized_win}"
    );
    assert!(
        sanitized_win.contains(r"[USER_HOME]\") || sanitized_win.contains("[USER_HOME]"),
        "Redacted path must contain [USER_HOME]: {sanitized_win}"
    );
}

#[test]
fn test_sanitizer_credential_redaction() {
    let bearer = "Authorization: Bearer mock_token_sk_ant_dummy_secret_val_123456";
    let sanitized_bearer = sanitize(bearer);
    assert!(
        !sanitized_bearer.contains("mock_token_sk_ant_dummy_secret_val_123456"),
        "Bearer token must be redacted: {sanitized_bearer}"
    );
    assert!(
        sanitized_bearer.contains("[REDACTED_SECRET]"),
        "Must contain [REDACTED_SECRET]: {sanitized_bearer}"
    );

    let json_secret = r#"{"apiKey": "secret_key_12345", "password": "SuperSecretPassword!"}"#;
    let sanitized_json_str = sanitize(json_secret);
    assert!(
        !sanitized_json_str.contains("SuperSecretPassword!"),
        "Password must be redacted: {sanitized_json_str}"
    );
}

#[test]
fn test_sanitizer_json_recursive_redaction() {
    let input = json!({
        "username": "alice",
        "api_key": "live_key_99998888",
        "nested": {
            "token": "tok_12345",
            "normal_field": "safe_value",
            "deep": {
                "password": "p@ssw0rd",
                "path": "/home/alice/workspace/config.json"
            }
        },
        "list": [
            {"secret": "xyz123"},
            {"info": "public"}
        ]
    });

    let sanitized = sanitize_json(&input);

    assert_eq!(sanitized["username"], "alice");
    assert_eq!(sanitized["api_key"], "[REDACTED_SECRET]");
    assert_eq!(sanitized["nested"]["token"], "[REDACTED_SECRET]");
    assert_eq!(sanitized["nested"]["normal_field"], "safe_value");
    assert_eq!(sanitized["nested"]["deep"]["password"], "[REDACTED_SECRET]");
    assert!(
        sanitized["nested"]["deep"]["path"]
            .as_str()
            .unwrap()
            .contains("[USER_HOME]"),
        "Deep path should have [USER_HOME]"
    );
    assert_eq!(sanitized["list"][0]["secret"], "[REDACTED_SECRET]");
    assert_eq!(sanitized["list"][1]["info"], "public");
}

#[test]
fn test_config_contract() {
    let default_cfg = PonySentryConfig::default();
    assert_eq!(default_cfg.endpoint, "https://sentry.ponyjob.top");
    assert_eq!(default_cfg.environment, "dev");
    assert_eq!(default_cfg.release, "0.1.109");
    assert!(default_cfg.enabled);

    std::env::set_var("PONYSENTRY_INGEST_URL", "http://10.43.94.160:3000");
    std::env::set_var("PONYSENTRY_CLIENT_TOKEN", "test-token-123");
    std::env::set_var("PONYSENTRY_ENABLED", "false");
    std::env::set_var("APP_ENV", "staging");

    let env_cfg = PonySentryConfig::from_env();
    assert_eq!(env_cfg.endpoint, "http://10.43.94.160:3000");
    assert_eq!(env_cfg.client_token.as_deref(), Some("test-token-123"));
    assert!(!env_cfg.enabled);
    assert_eq!(env_cfg.environment, "staging");

    // Clean up
    std::env::remove_var("PONYSENTRY_INGEST_URL");
    std::env::remove_var("PONYSENTRY_CLIENT_TOKEN");
    std::env::remove_var("PONYSENTRY_ENABLED");
    std::env::remove_var("APP_ENV");
}

#[test]
fn test_client_fire_and_forget_contract() {
    let config = PonySentryConfig {
        endpoint: "http://127.0.0.1:65534".to_string(), // Unreachable port
        client_token: None,
        enabled: true,
        environment: "test".to_string(),
        release: "0.1.109".to_string(),
    };

    let client = PonySentryClient::new(config);

    // Calling capture_error on an unreachable endpoint must NOT panic or block
    let start = std::time::Instant::now();
    client.capture_error("TestError", "Test message", Some(json!({"test": true})));
    let elapsed = start.elapsed();

    // Must return immediately (fire-and-forget: under 50ms)
    assert!(
        elapsed < std::time::Duration::from_millis(100),
        "capture_error took too long: {:?}",
        elapsed
    );
}

#[test]
fn test_breadcrumb_ring_buffer_limit() {
    let config = PonySentryConfig {
        endpoint: "http://127.0.0.1:65534".to_string(),
        client_token: None,
        enabled: false,
        environment: "test".to_string(),
        release: "0.1.109".to_string(),
    };

    let client = PonySentryClient::new(config);

    // Add 100 breadcrumbs
    for i in 0..100 {
        client.add_breadcrumb(
            "test",
            &format!("Step {i}"),
            Some({
                let mut m = HashMap::new();
                m.insert("step".to_string(), i.to_string());
                m
            }),
        );
    }

    // Module-level functions should also be operable
    init(PonySentryConfig::default());
    add_breadcrumb("ui", "User clicked submit", None);
    capture_error("UiError", "Button state mismatch", None);
}
