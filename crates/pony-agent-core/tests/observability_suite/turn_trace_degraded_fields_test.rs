//! C1（P1-2 收尾）红相：`TurnTraceRecord` 新增 `degraded` / `tool_failures` 并保证向后兼容。
//!
//! 契约真源：P1-2（把 OTel span 上的 `degraded` / `tool_failures` 带到本地会话数据，供 UI 显示）。
//!
//! 要求：
//! 1. 新字段 `degraded: Option<bool>`：**仅 `true` 时**输出（`None`/`false` 不写键）；
//! 2. 新字段 `tool_failures: Option<usize>`：**仅 > 0 时**输出（`None`/`0` 不写键）；
//! 3. wire 键名 camelCase：`degraded` / `toolFailures`；
//! 4. **向后兼容硬断言**：不含这两个键的旧记录 JSON 必须能反序列化，且两字段为 `None`；
//! 5. 现有字段一个不少、类型不变（既有键集与取值逐字保留）。
//!
//! 本文件只测纯序列化面：不触网、不读写会话存储。

use pony_agent_core::agent::session::TurnTraceRecord;
use serde_json::{json, Value};

/// 既有可序列化键集（改动前实测；用于锁「现有字段一个不少」）。
const EXISTING_KEYS: [&str; 34] = [
    "buildContextObservation",
    "cacheHitInputTokens",
    "emittedAtMs",
    "error",
    "eventId",
    "eventType",
    "eventVersion",
    "fallbackReason",
    "firstTokenLatencyMs",
    "hookTraceRecords",
    "inputText",
    "inputTokens",
    "outputText",
    "outputTokens",
    "phase",
    "providerCallRecords",
    "providerMode",
    "providerModel",
    "providerName",
    "providerProtocol",
    "providerRequestedName",
    "providerSource",
    "reasoningTokens",
    "sequence",
    "sessionId",
    "sessionSummary",
    "title",
    "toolActivities",
    "totalTokens",
    "traceSteps",
    "traceTimeline",
    "turnDurationMs",
    "turnId",
    "updatedAt",
];

/// 构造一条最小记录（其余字段走 `Default`）。
fn record_with(
    degraded: Option<bool>,
    tool_failures: Option<usize>,
) -> TurnTraceRecord {
    TurnTraceRecord {
        turn_id: "turn-c1".to_string(),
        title: "Turn C1".to_string(),
        phase: "completed".to_string(),
        degraded,
        tool_failures,
        updated_at: 1_700_000_000,
        ..Default::default()
    }
}

fn serialized(record: &TurnTraceRecord) -> Value {
    serde_json::to_value(record).expect("C1：TurnTraceRecord 必须可序列化")
}

// ── 契约 1/2/3：仅在「有意义」时以 camelCase 输出 ──────────────────────────

#[test]
fn degraded_and_failures_serialize_as_camel_case_when_meaningful() {
    // Act
    let value = serialized(&record_with(Some(true), Some(3)));
    println!("[C1] degraded=true failures=3 ⇒ {value}");

    // Assert
    assert_eq!(
        value.get("degraded"),
        Some(&json!(true)),
        "C1：`degraded: Some(true)` 必须以 camelCase 键 `degraded` 输出，实际 {value}"
    );
    assert_eq!(
        value.get("toolFailures"),
        Some(&json!(3)),
        "C1：`tool_failures: Some(3)` 必须以 camelCase 键 `toolFailures` 输出，实际 {value}"
    );
    assert!(
        value.get("tool_failures").is_none(),
        "C1：不得同时出现 snake_case 键 `tool_failures`，实际 {value}"
    );
}

#[test]
fn false_and_zero_and_none_omit_keys() {
    // Act + Assert：Some(false) / Some(0) 都不得写键（避免 UI 噪音）。
    let false_and_zero = serialized(&record_with(Some(false), Some(0)));
    println!("[C1] degraded=false failures=0 ⇒ {false_and_zero}");
    assert!(
        false_and_zero.get("degraded").is_none(),
        "C1：`degraded: Some(false)` 不得写键（仅 true 输出），实际 {false_and_zero}"
    );
    assert!(
        false_and_zero.get("toolFailures").is_none(),
        "C1：`tool_failures: Some(0)` 不得写键（仅 >0 输出），实际 {false_and_zero}"
    );

    // None / None 同样不得写键。
    let unset = serialized(&record_with(None, None));
    println!("[C1] degraded=None failures=None ⇒ {unset}");
    assert!(
        unset.get("degraded").is_none(),
        "C1：`degraded: None` 不得写键，实际 {unset}"
    );
    assert!(
        unset.get("toolFailures").is_none(),
        "C1：`tool_failures: None` 不得写键，实际 {unset}"
    );
}

// ── 契约 4：旧记录反序列化（向后兼容硬断言） ──────────────────────────────

#[test]
fn legacy_json_without_new_fields_deserializes() {
    // Arrange：改动前的历史记录形状（完全没有 degraded / toolFailures）。
    let legacy = json!({
        "turnId": "turn-legacy",
        "title": "Legacy Turn",
        "phase": "completed",
        "updatedAt": 1_690_000_000u64,
        "sessionId": "sess-legacy",
        "traceSteps": [],
        "traceTimeline": [],
        "toolActivities": []
    });

    // Act
    let record: TurnTraceRecord = serde_json::from_value(legacy)
        .expect("C1：不含新字段的旧记录必须能反序列化（向后兼容硬要求）");

    // Assert
    assert_eq!(record.turn_id, "turn-legacy");
    assert_eq!(
        record.degraded, None,
        "C1：旧记录反序列化后 degraded 必须为 None，实际 {:?}",
        record.degraded
    );
    assert_eq!(
        record.tool_failures, None,
        "C1：旧记录反序列化后 tool_failures 必须为 None，实际 {:?}",
        record.tool_failures
    );
}

#[test]
fn round_trip_preserves_payload() {
    // Arrange
    let record = record_with(Some(true), Some(2));

    // Act
    let value = serialized(&record);
    let back: TurnTraceRecord =
        serde_json::from_value(value.clone()).expect("C1：新记录必须可反序列化（round-trip）");
    let value_after = serialized(&back);

    // Assert
    assert_eq!(
        value, value_after,
        "C1：round-trip 后载荷必须逐字一致（含 degenerate 边界字段）"
    );
    assert_eq!(back.degraded, Some(true));
    assert_eq!(back.tool_failures, Some(2));
}

// ── 契约 5：既有字段一个不少、类型不变 ────────────────────────────────────

#[test]
fn existing_field_set_is_unchanged() {
    // Act：默认记录的键集 = 既有字段集合（新字段在 None 下不输出）。
    let value = serialized(&TurnTraceRecord::default());
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("C1：TurnTraceRecord 必须序列化为 JSON 对象，实际 {value}"));
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();

    // Assert 1：既有键一个不少、也不多出未声明的键。
    let mut expected = EXISTING_KEYS.to_vec();
    expected.sort_unstable();
    assert_eq!(
        keys, expected,
        "C1：默认记录的序列化键集必须与改动前逐字一致（现有字段一个不少、无多余键）"
    );

    // Assert 2：关键字段类型不变（抽样钉死类型，防「同名换类型」破坏前端）。
    assert!(object["turnId"].is_string(), "C1：turnId 必须是字符串");
    assert!(object["title"].is_string(), "C1：title 必须是字符串");
    assert!(object["phase"].is_string(), "C1：phase 必须是字符串");
    assert!(object["updatedAt"].is_u64(), "C1：updatedAt 必须是无符号整数");
    assert!(
        object["traceSteps"].is_array(),
        "C1：traceSteps 必须是数组"
    );
    assert!(
        object["toolActivities"].is_array(),
        "C1：toolActivities 必须是数组"
    );
}
