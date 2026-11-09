//! PA-171 **C4 红相**验收：失败轮归因真值（`provider_source` / `provider_mode` /
//! `fallback_reason` 不再硬编码 None，并新增审计可判的 `error_class`）。
//!
//! 契约真值源：`management/task-system/03_TASKS/PA-171-goal-round-failure-resilience.md` C4。
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。
//!
//! 缺陷面（已取证，`turn_flow.rs:581-583`）：`emit_stream_failed` 把
//! `provider_source` / `provider_mode` / `fallback_reason` **硬编码为 None** ⇒ 失败轮在 trace
//! 里完全不可归因，审计机只能看到"failed"；且事件里没有 `error_class`，无法区分
//! timeout / rate_limited / quota / auth / network。
//!
//! 断言口径（不锁实现细节）：
//! 1. 失败事件（`turn:failed`）序列化后三字段**非 null 且非空**（不再占位 None）；
//! 2. 事件携带 `error_class`（camelCase `errorClass` 亦可），取值与错误文本一致：
//!    timeout→`timeout`、429→`rate_limited`、quota→`quota`、401→`auth`、
//!    connection reset→`network`、未知→`other`；
//! 3. 失败语义不变：`kind == "failed"`、`error` 原样保留（本卡不得吞掉原始错误）。
//!
//! 接线提示（TASK-3-WIRE）：若 Executor 为归因**新增形参**（如 `error_class: ErrorClass`），
//! 请与 Lead 协商后同步本文件 `emit_failed` 的**唯一调用点**；三字段非 null 与
//! error_class 映射的断言语义不得放宽。本文件只走纯函数 + 本地 sink，不联网、不起 provider。

use std::sync::Mutex;

use pony_agent_core::agent::hooks::HookTraceRecord;
use pony_agent_core::agent::provider::BuildContextObservation;
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::session::TraceTimelineEntry;
use pony_agent_core::agent::telemetry::{
    ProviderCallCacheRecord, TurnToolActivity, TurnTraceStep,
};
use pony_agent_core::agent::turn_flow::{emit_stream_failed, ProviderEventMeta, TurnEventSink};

/// 失败轮必然携带的 provider 归因元数据（真实运行时的形状）。
fn provider_meta() -> ProviderEventMeta {
    ProviderEventMeta {
        requested_name: "magpie".to_string(),
        provider_name: "kimi".to_string(),
        protocol: "openai-completions".to_string(),
        model: "k3.1-pro".to_string(),
        context_window_tokens: Some(131_072),
    }
}

/// 捕获 sink：把 `(事件名, 载荷)` 收进内存。
#[derive(Default)]
struct CapturingSink {
    events: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl TurnEventSink for CapturingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events
            .lock()
            .expect("PA-171：capture sink mutex poisoned")
            .push((name.to_string(), payload));
    }
}

/// 调一次 `emit_stream_failed`，返回捕获到的 `(事件名, 事件)`。
fn emit_failed(error: &str) -> (String, TurnStreamEvent) {
    let sink = CapturingSink::default();
    let meta = provider_meta();
    let _envelope = emit_stream_failed(
        &sink,
        "turn-pa171-c4".to_string(),
        Some(&meta),                              // provider_meta（真实归因来源）
        Vec::<TurnTraceStep>::new(),              // trace_steps
        None::<Vec<TurnToolActivity>>,            // tool_activities
        None,                                     // first_token_latency_ms
        None,                                     // turn_duration_ms
        None::<BuildContextObservation>,          // build_context_observation
        None::<Vec<TraceTimelineEntry>>,          // trace_timeline
        None::<Vec<ProviderCallCacheRecord>>,     // provider_call_records
        None::<Vec<HookTraceRecord>>,             // hook_trace_records
        error.to_string(),                        // error
        None,                                     // session_id
        None,                                     // partial_text
    );

    let mut events = sink
        .events
        .lock()
        .expect("PA-171：capture sink mutex poisoned");
    assert_eq!(
        events.len(),
        1,
        "PA-171：一次 emit_stream_failed 必须且只能产生 1 个事件，实际 {}",
        events.len()
    );
    events.remove(0)
}

/// 事件 → JSON（`TurnStreamEvent` 未实现 `Debug`，用其自身 Serialize 输出复核）。
fn event_json(event: &TurnStreamEvent) -> serde_json::Value {
    serde_json::to_value(event).expect("PA-171：TurnStreamEvent 必须可序列化（审计机读的就是它）")
}

/// 兼容取字段：serde 对 `TurnStreamEvent` 用 `rename_all = "camelCase"`，
/// 但归因字段的命名约定可能被实现选为 snake_case，两种拼写都接受。
fn field<'a>(value: &'a serde_json::Value, snake: &str, camel: &str) -> Option<&'a serde_json::Value> {
    value.get(snake).or_else(|| value.get(camel))
}

fn assert_non_placeholder(value: &serde_json::Value, snake: &str, camel: &str, ticket: &str) {
    let raw = field(value, snake, camel);
    let text = raw
        .and_then(|value| value.as_str())
        .unwrap_or_else(|| panic!("{ticket}：字段 {snake} 缺失或不是字符串，实际 {raw:?}"));
    assert!(
        !text.trim().is_empty(),
        "{ticket}：字段 {snake} 不得为空串占位，实际 {text:?}"
    );
    assert!(
        !matches!(text.trim().to_ascii_lowercase().as_str(), "none" | "null" | "unknown"),
        "{ticket}：字段 {snake} 不得用占位值 {text:?} 冒充真实归因"
    );
}

// ── 断言 1：三字段非占位 None ──────────────────────────────────────────────────────

/// C4-a：失败轮事件必须落真实 `provider_source` / `provider_mode` / `fallback_reason`。
#[test]
fn c4_failed_turn_event_attributes_provider_source_mode_and_fallback_reason() {
    // Arrange
    let error = "provider 返回错误状态：429 Too Many Requests；耗时=12ms；响应正文：{\"error\":{\"type\":\"exceeded_current_quota_error\"}}";

    // Act
    let (name, event) = emit_failed(error);
    let value = event_json(&event);
    println!("[PA-171 C4] failed event = {value}");

    // Assert：事件仍可识别为失败轮（失败语义不得被归因改动破坏）。
    assert_eq!(name, "turn:failed", "C4：事件名必须仍为 turn:failed，实际 {name}");
    assert_eq!(event.kind, "failed", "C4：kind 必须仍为 failed");
    assert_eq!(
        event.error.as_deref(),
        Some(error),
        "C4：失败事件必须原样保留原始错误文本（审计与文案都依赖它）"
    );

    // Assert：三字段不再是占位 None。
    assert_non_placeholder(&value, "provider_source", "providerSource", "C4");
    assert_non_placeholder(&value, "provider_mode", "providerMode", "C4");
    assert_non_placeholder(&value, "fallback_reason", "fallbackReason", "C4");

    // Assert：归因值与 provider_meta 一致（不得凭空编造另一个 provider）。
    let source = field(&value, "provider_source", "providerSource")
        .and_then(|item| item.as_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        source.contains("magpie") || source.contains("kimi"),
        "C4：provider_source 必须是本轮真实来源（magpie/kimi），实际 {source:?}"
    );
    // provider_mode 只锁「非占位」，不锁具体取值：失败轮用 live/error/degraded 都能自洽，
    // 过度约束会把实现细节焊死（审计语义只要求可判）。
}

// ── 断言 2：error_class 与错误文本一致 ────────────────────────────────────────────

/// C4-b：`error_class` 真值表 —— 审计机据此区分 timeout / rate_limited / quota / auth / network。
#[test]
fn c4_error_class_matches_error_text() {
    let cases: [(&str, &str); 6] = [
        (
            "调用 provider 失败：operation timed out after 45002ms",
            "timeout",
        ),
        (
            "provider 返回错误状态：429 Too Many Requests；耗时=12ms；响应正文：{\"error\":{\"type\":\"rate_limit_exceeded\"}}",
            "rate_limited",
        ),
        (
            "{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit, resets at 06:01 UTC\"}}",
            "quota",
        ),
        ("provider 返回错误状态：401 Unauthorized；耗时=5ms", "auth"),
        ("connection reset by peer", "network"),
        ("boom: unexpected local invariant violation", "other"),
    ];

    for (error, expected) in cases {
        // Act
        let (_name, event) = emit_failed(error);
        let value = event_json(&event);
        let actual = field(&value, "error_class", "errorClass")
            .and_then(|item| item.as_str())
            .map(str::to_string);

        // Assert
        assert_eq!(
            actual.as_deref(),
            Some(expected),
            "C4：错误文本 {error:?} 的 error_class 必须是 {expected:?}（审计机唯一判据），实际 {actual:?}"
        );
    }
}

/// C4-c：`error_class` 必须**随事件序列化下发**（审计机读的是 wire 形态），
/// 不允许只在内存结构里存在而 JSON 里缺席。
#[test]
fn c4_error_class_is_present_on_the_wire() {
    // Arrange
    let error = "provider 返回错误状态：429 Too Many Requests；耗时=9ms；响应正文：{}";

    // Act
    let (_name, event) = emit_failed(error);
    let value = event_json(&event);

    // Assert
    assert!(
        field(&value, "error_class", "errorClass").is_some(),
        "C4：失败事件 wire 上必须存在 error_class/errorClass 键，实际 JSON = {value}"
    );
}
