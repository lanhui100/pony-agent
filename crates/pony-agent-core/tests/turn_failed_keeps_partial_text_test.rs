//! P0-3 第二刀红相：失败轮**保留已产出正文**（trace 报告：失败 turn 只剩 "This turn failed."，正文全丢）。
//!
//! 契约真源：trace 报告 P0-3。
//! 被测纯函数：`agent::turn_flow::emit_stream_failed`（pub），本轮追加最后一个参数
//! `partial_text: Option<String>`（已产出但未完成的正文）。
//!
//! 语义：
//! 1. `Some(正文)` ⇒ 事件 `text` **包含**该正文，且仍可识别为失败（`kind=="failed"`、
//!    `phase=="failed"`、`error` 非空）；
//! 2. `None` ⇒ `text` 逐字等于现状 `"This turn failed."`（**兼容锁**，防历史断言破）；
//! 3. `Some(全空白)` ⇒ 视为无内容，回落 `"This turn failed."`；
//! 4. `Some(超长)` ⇒ `text` 有硬上界（≤ 8192 字节）且带截断标记。
//!
//! 本文件只测纯函数 + 本地 sink：不起网络、不读全局 provider。

use std::sync::Mutex;

use pony_agent_core::agent::hooks::HookTraceRecord;
use pony_agent_core::agent::provider::BuildContextObservation;
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::session::TraceTimelineEntry;
use pony_agent_core::agent::telemetry::{
    ProviderCallCacheRecord, TurnToolActivity, TurnTraceStep,
};
use pony_agent_core::agent::turn_flow::{emit_stream_failed, TurnEventSink};

/// 现状文案（兼容锁的既有契约值）。
const LEGACY_FAILED_TEXT: &str = "This turn failed.";

/// 失败轮事件正文的硬上界（实现可更小；本断言只锁「有上界 + 带标记」）。
const PARTIAL_TEXT_LIMIT_BYTES: usize = 8 * 1024;

/// 捕获 sink：把 `(事件名, 载荷)` 收进内存，供断言事件正文。
#[derive(Default)]
struct CapturingSink {
    events: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl TurnEventSink for CapturingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events
            .lock()
            .expect("capture sink mutex poisoned")
            .push((name.to_string(), payload));
    }
}

/// 调一次 `emit_stream_failed`，返回捕获到的 `(事件名, 事件)`。
///
/// 未参与断言的入参一律传 `None` / 空集合（最小可构造入参）。
fn emit_failed(partial_text: Option<String>) -> (String, TurnStreamEvent) {
    let sink = CapturingSink::default();
    let _envelope = emit_stream_failed(
        &sink,
        "turn-p0-3".to_string(),
        None,                                       // provider_meta
        Vec::<TurnTraceStep>::new(),                // trace_steps
        None::<Vec<TurnToolActivity>>,              // tool_activities
        None,                                       // first_token_latency_ms
        None,                                       // turn_duration_ms
        None::<BuildContextObservation>,            // build_context_observation
        None::<Vec<TraceTimelineEntry>>,            // trace_timeline
        None::<Vec<ProviderCallCacheRecord>>,       // provider_call_records
        None::<Vec<HookTraceRecord>>,               // hook_trace_records
        "provider exploded".to_string(),            // error
        None,                                       // session_id
        partial_text,                               // P0-3：已产出正文（本轮新增）
    );

    let mut events = sink.events.lock().expect("capture sink mutex poisoned");
    assert_eq!(
        events.len(),
        1,
        "P0-3：一次 emit_stream_failed 必须且只能产生 1 个事件，实际 {}",
        events.len()
    );
    events.remove(0)
}

/// 事件摘要（`TurnStreamEvent` 未实现 `Debug`；用它自身的 Serialize 输出便于复核）。
fn event_summary(event: &TurnStreamEvent) -> String {
    serde_json::to_string(event).unwrap_or_else(|error| format!("<serialize failed: {error}>"))
}

/// 失败语义锁：无论正文如何，事件必须仍可被识别为失败轮。
fn assert_still_failed(name: &str, event: &TurnStreamEvent) {
    assert_eq!(
        name, "turn:failed",
        "P0-3：事件名必须仍为 turn:failed，实际 `{name}`"
    );
    assert_eq!(
        event.kind, "failed",
        "P0-3：事件 kind 必须仍为 failed，实际 `{}`",
        event.kind
    );
    assert_eq!(
        event.phase.as_deref(),
        Some("failed"),
        "P0-3：事件 phase 必须仍为 failed，实际 {:?}",
        event.phase
    );
    assert!(
        event.error.as_deref().map(str::trim).is_some_and(|text| !text.is_empty()),
        "P0-3：失败事件必须仍带非空 error，实际 {:?}",
        event.error
    );
}

#[test]
fn partial_text_is_kept_in_failed_event() {
    // Arrange
    let partial = "这是已产出的内容";

    // Act
    let (name, event) = emit_failed(Some(partial.to_string()));
    println!("[P0-3b] partial=Some(正文) ⇒ event={}", event_summary(&event));

    // Assert
    assert_still_failed(&name, &event);
    let text = event
        .text
        .as_deref()
        .unwrap_or_else(|| panic!("P0-3：失败事件必须带 text，实际 {}", event_summary(&event)));
    assert!(
        text.contains(partial),
        "P0-3：失败事件正文必须**保留**已产出内容 `{partial}`，实际 text={text:?}"
    );
    assert_ne!(
        text, LEGACY_FAILED_TEXT,
        "P0-3：有已产出正文时不得再退化成占位文案，实际 text={text:?}"
    );
}

#[test]
fn without_partial_text_falls_back_to_legacy_message() {
    // Act
    let (name, event) = emit_failed(None);
    println!("[P0-3b] partial=None ⇒ event={}", event_summary(&event));

    // Assert（兼容锁：历史冻结断言逐字依赖该文案）
    assert_still_failed(&name, &event);
    assert_eq!(
        event.text.as_deref(),
        Some(LEGACY_FAILED_TEXT),
        "P0-3：无已产出正文时 text 必须逐字等于 `{LEGACY_FAILED_TEXT}`（兼容锁）"
    );
}

#[test]
fn blank_partial_text_falls_back_to_legacy_message() {
    // Act
    let (name, event) = emit_failed(Some("   \n\t  ".to_string()));
    println!("[P0-3b] partial=Some(全空白) ⇒ event={}", event_summary(&event));

    // Assert
    assert_still_failed(&name, &event);
    assert_eq!(
        event.text.as_deref(),
        Some(LEGACY_FAILED_TEXT),
        "P0-3：全空白正文必须视为无内容并回落 `{LEGACY_FAILED_TEXT}`"
    );
}

#[test]
fn oversized_partial_text_is_bounded_and_marked() {
    // Arrange：10KB 纯 ASCII，超上界。
    let huge = "z".repeat(10 * 1024);

    // Act
    let (name, event) = emit_failed(Some(huge));
    println!(
        "[P0-3b] partial=Some(10KB ASCII) ⇒ text_len={:?}",
        event.text.as_ref().map(|text| text.len())
    );

    // Assert
    assert_still_failed(&name, &event);
    let text = event
        .text
        .as_deref()
        .unwrap_or_else(|| panic!("P0-3：超长正文仍必须写出 text（截断而非丢弃），实际 {}", event_summary(&event)));
    assert!(
        text.len() <= PARTIAL_TEXT_LIMIT_BYTES,
        "P0-3：失败事件正文必须有硬上界（≤ {PARTIAL_TEXT_LIMIT_BYTES} 字节），实际 {}",
        text.len()
    );
    let lowered = text.to_lowercase();
    assert!(
        lowered.contains("truncat") || text.contains("截断"),
        "P0-3：超长正文必须带截断语义标记（truncated/截断），实际尾部={:?}",
        &text[text.len().saturating_sub(40)..]
    );
    assert!(
        !text.trim().is_empty(),
        "P0-3：截断不得把正文清空"
    );
}
