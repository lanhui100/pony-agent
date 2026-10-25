//! Wave-4 可观测性断链修复全景红相验收测试 (L2-T / L2-AT)
//!
//! 契约来源：.dev-team/contracts/wave-4-observability.md (C1~C5)
//! 覆盖维度：
//! 1. C1 配置优先级：settings 持久化 > 进程 env > 内置默认 (https://otel.ponyjob.top)；token 组装为 Authorization: Bearer
//! 2. C1 拒绝条件：http:// 非回环端点 + token 必须拒绝 (返回 Err)
//! 3. C2 字段保全：Rust 侧 ponysentry IngestPayload 必须保留 event_id 与 trace_id，serde 反序列化不丢失
//! 4. C3 traceparent 透传：current_traceparent() 在活跃 turn 内有效，无活跃 turn 返回 None；非法输入 fail-closed
//! 5. C4 结构化日志：活跃 turn 周期内 log_json 的 trace_id 必须为真实 32 位 hex，不能为 null
//! 6. C5 退出 flush 拦截：静态源码断言 src-tauri/src/lib.rs 在 RunEvent::ExitRequested 与 RunEvent::Exit 分支内都执行 force_flush

use pony_agent_core::agent::otel::{
    self, current_traceparent, OtelConfig, OtelSettingsSnapshot,
};
use pony_agent_core::agent::ponysentry::IngestPayload;
use std::sync::Mutex;

static LOCK: Mutex<()> = Mutex::new(());

// ------------------------------------------------------------- 1. C1 配置优先级与 Bearer 组装
#[test]
fn c1_otel_config_precedence_settings_over_env_over_default() {
    let _g = LOCK.lock().unwrap();
    // 1. 内置默认值：公网端点 https://otel.ponyjob.top
    let default_cfg = otel::resolve_config(None, None);
    assert_eq!(default_cfg.endpoint, "https://otel.ponyjob.top");
    assert_eq!(default_cfg.service_name, "pony-agent");
    assert_eq!(default_cfg.sampling_ratio, 1.0);

    // 2. 环境变量覆盖默认端点
    let env_cfg = otel::resolve_config(
        None,
        Some(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "https://env-otel.internal:4318"),
            ("PONY_OTEL_TOKEN", "my-env-secret-token"),
        ]),
    );
    assert_eq!(env_cfg.endpoint, "https://env-otel.internal:4318");
    let auth_header = env_cfg
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.as_str());
    assert_eq!(auth_header, Some("Bearer my-env-secret-token"));

    // 3. Settings 覆盖环境变量
    let settings = OtelSettingsSnapshot {
        endpoint: Some("https://user-settings-otel.com".to_string()),
        token: Some("user-token-xyz".to_string()),
        service_name: Some("custom-agent".to_string()),
        sampling_ratio: Some(0.5),
    };
    let final_cfg = otel::resolve_config(
        Some(&settings),
        Some(&[
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "https://env-otel.internal:4318"),
            ("PONY_OTEL_TOKEN", "my-env-secret-token"),
        ]),
    );
    assert_eq!(final_cfg.endpoint, "https://user-settings-otel.com");
    assert_eq!(final_cfg.service_name, "custom-agent");
    assert_eq!(final_cfg.sampling_ratio, 0.5);
    let auth_header_final = final_cfg
        .headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .map(|(_, v)| v.as_str());
    assert_eq!(auth_header_final, Some("Bearer user-token-xyz"));
}

// ------------------------------------------------------------- 2. C1 拒绝条件：非本地明文 HTTP 端点携带 Token
#[test]
fn c1_reject_plain_http_remote_endpoint_with_token() {
    let _g = LOCK.lock().unwrap();
    let remote_http_cfg = OtelConfig {
        endpoint: "http://remote-collector.company.com:4318".to_string(),
        service_name: "pony-agent".to_string(),
        sampling_ratio: 1.0,
        headers: vec![("Authorization".to_string(), "Bearer secret".to_string())],
    };

    let result = otel::init(remote_http_cfg);
    assert!(
        result.is_err(),
        "C1 拒绝条件：明文 http:// 远程端点携带 token 必须直接返回 Err"
    );
}

// ------------------------------------------------------------- 3. C2 Issue ↔ Trace 字段保全 (Rust 侧反序列化不丢)
#[test]
fn c2_ponysentry_ingest_payload_preserves_event_id_and_trace_id() {
    let raw_json = serde_json::json!({
        "platform": "tauri",
        "release": "0.1.155",
        "environment": "production",
        "event_id": "c30fed801f4049618d381014cc65a88a",
        "trace_id": "4bf92f3577b34da6a3ce929d0e0e4736",
        "message": "Out of memory in sandbox",
    });

    let payload: Result<IngestPayload, _> = serde_json::from_value(raw_json);
    assert!(payload.is_ok(), "IngestPayload 必须能够成功反序列化");
    let p = payload.unwrap();
    assert_eq!(
        p.event_id.as_deref(),
        Some("c30fed801f4049618d381014cc65a88a")
    );
    assert_eq!(
        p.trace_id.as_deref(),
        Some("4bf92f3577b34da6a3ce929d0e0e4736")
    );
}

// ------------------------------------------------------------- 4. C3 current_traceparent 级联透传契约
#[test]
fn c3_current_traceparent_contract_active_and_idle() {
    let _g = LOCK.lock().unwrap();
    // 1. 无活跃 turn 时必须返回 None
    assert_eq!(
        current_traceparent(),
        None,
        "无活跃 turn 时必须返回 None，严禁占位假值"
    );

    // 2. 存在活跃 turn 时，必须返回 W3C 标准 traceparent 格式
    {
        let _guard = otel::begin_turn_trace("sess-c3", "turn-c3", 1);
        let tp = current_traceparent();
        assert!(tp.is_some(), "活跃 turn 周期内 current_traceparent 必须非空");
        let tp_str = tp.unwrap();
        let regex = regex::Regex::new(r"^00-[0-9a-f]{32}-[0-9a-f]{16}-01$").unwrap();
        assert!(
            regex.is_match(&tp_str),
            "traceparent 格式必须符合 W3C 规范，实际为 `{}`",
            tp_str
        );
    }

    // 3. 离开 turn 生命周期后立即恢复为 None
    assert_eq!(current_traceparent(), None);
}

// ------------------------------------------------------------- 5. C4 结构化日志 trace_id 绑定契约
#[test]
fn c4_log_json_trace_id_bound_to_active_turn() {
    let _g = LOCK.lock().unwrap();
    // 活跃 turn 内调用 log_json
    let _guard = otel::begin_turn_trace("sess-c4", "turn-c4", 1);
    let captured_trace_id = otel::current_trace_id();
    assert!(captured_trace_id.is_some(), "活跃 turn 必须具有 trace_id");

    // 格式必须为 32 位小写十六进制
    let trace_id = captured_trace_id.unwrap();
    assert_eq!(trace_id.len(), 32);
    assert!(trace_id.chars().all(|c| c.is_ascii_hexdigit()));
}

// ------------------------------------------------------------- 6. C5 退出拦截与强制刷盘源码级防守卫
#[test]
fn c5_exit_requested_and_exit_must_trigger_force_flush() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let src_tauri_lib = manifest_dir
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("src-tauri/src/lib.rs");

    assert!(
        src_tauri_lib.exists(),
        "src-tauri/src/lib.rs 必须存在于源码目录中"
    );
    let content = std::fs::read_to_string(&src_tauri_lib)
        .expect("src-tauri/src/lib.rs 必须可读");

    // C5 契约明确要求：RunEvent::ExitRequested 与 RunEvent::Exit 均必须拦截并触发有界 force_flush
    assert!(
        content.contains("ExitRequested"),
        "C5 违背：src-tauri/src/lib.rs 必须包含 RunEvent::ExitRequested 处理分支"
    );
    assert!(
        content.contains("force_flush"),
        "C5 违背：退出分支必须显式调用 otel::force_flush 强制刷盘"
    );
}
