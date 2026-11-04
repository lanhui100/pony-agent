//! P0-3 收尾红相：turn span 记录**失败原因**（trace `pa_01a1a134` turn-3 只有 status=failed、无归因）。
//!
//! 契约真源：trace 报告 P0-3「失败归因缺失」+ Lead 裁决 **方案 A**：走
//! `TurnSpanInput` 新字段 `failed_reason: Option<String>` + 纯映射 `turn_span_attributes`，
//! 由 `TurnTraceGuard::drop` 复用同一映射（不另写 `set_attribute`，避免两处口径漂移）。
//!
//! 验收：
//! 1. `Some("tool_failed: timeout")` ⇒ 属性含 `agent.turn.failed_reason`，值可追溯到该串；
//! 2. `None` / `Some("   ")`（全空白）⇒ 该键**缺席**（禁止空串/占位）；
//! 3. 非失败 phase + 无 reason ⇒ 键缺席（`agent.turn.phase` 仍按既有映射输出）；
//! 4. **兼容锁**：既有属性集（键序 + 值）在加字段后逐字不变；
//! 5. `failed_reason` 含敏感串 ⇒ 属性值必须已脱敏（复用 `ponysentry::sanitize` 口径，与 P0-1 一致）。
//!
//! 本文件只测**纯函数**：不起网络、不 init 导出器、不读全局 provider。

use pony_agent_core::agent::otel::{turn_span_attributes, AttrValue, TurnSpanInput};

/// 新键（契约字面量，不从实现常量取，避免「测试跟着实现改」）。
const KEY_FAILED_REASON: &str = "agent.turn.failed_reason";

/// 既有键（顺序与 `turn_span_attributes` 的既有产出顺序一致 —— 兼容锁的一部分）。
const EXISTING_KEYS: [&str; 6] = [
    "langfuse.session.id",
    "langfuse.trace.name",
    "gen_ai.operation.name",
    "agent.turn.id",
    "agent.turn.sequence",
    "agent.turn.phase",
];

const KEY_USER_ID: &str = "langfuse.user.id";

/// 构造 turn span 输入（`failed_reason` 是 P0-3 新增字段）。
fn turn_input(phase: &str, user_id: Option<&str>, failed_reason: Option<&str>) -> TurnSpanInput {
    TurnSpanInput {
        session_id: "sess-p0-3".to_string(),
        user_id: user_id.map(str::to_string),
        turn_id: "turn-p0-3".to_string(),
        sequence: 3,
        phase: phase.to_string(),
        failed_reason: failed_reason.map(str::to_string),
    }
}

/// 取属性键序列（顺序敏感）。
fn keys(attributes: &[(String, AttrValue)]) -> Vec<&str> {
    attributes.iter().map(|(key, _)| key.as_str()).collect()
}

/// 取字符串属性值。
fn attr_str<'a>(attributes: &'a [(String, AttrValue)], key: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(name, _)| name == key)
        .and_then(|(_, value)| match value {
            AttrValue::Str(text) => Some(text.as_str()),
            _ => None,
        })
}

// ── 契约 1：失败原因被记录 ────────────────────────────────────────────────

#[test]
fn failed_reason_is_mapped_to_turn_span_attribute() {
    // Arrange
    let reason = "tool_failed: timeout";
    let input = turn_input("failed", None, Some(reason));

    // Act
    let attributes = turn_span_attributes(&input);
    println!("[P0-3c] phase=failed reason=Some ⇒ attrs={attributes:?}");

    // Assert
    let value = attr_str(&attributes, KEY_FAILED_REASON).unwrap_or_else(|| {
        panic!(
            "P0-3：失败 turn 必须记录 {KEY_FAILED_REASON}（trace 归因缺失即本卡要修的问题）；实际属性={attributes:?}"
        )
    });
    assert!(
        value.contains(reason),
        "P0-3：{KEY_FAILED_REASON} 必须可追溯到失败原因 `{reason}`，实际值={value:?}"
    );
    assert_eq!(
        attr_str(&attributes, "agent.turn.phase"),
        Some("failed"),
        "兼容锁：agent.turn.phase 必须仍为 failed，实际属性={attributes:?}"
    );
}

// ── 契约 2：无原因 ⇒ 键缺席 ───────────────────────────────────────────────

#[test]
fn absent_or_blank_reason_omits_key() {
    // Act + Assert（None）
    let none_attrs = turn_span_attributes(&turn_input("failed", None, None));
    println!("[P0-3c] reason=None ⇒ keys={:?}", keys(&none_attrs));
    assert!(
        none_attrs.iter().all(|(key, _)| key != KEY_FAILED_REASON),
        "P0-3：reason=None 时不得写出 {KEY_FAILED_REASON}，实际属性={none_attrs:?}"
    );

    // Act + Assert（全空白 ⇒ 视为无原因）
    let blank_attrs = turn_span_attributes(&turn_input("failed", None, Some("   \n\t ")));
    println!("[P0-3c] reason=Some(全空白) ⇒ keys={:?}", keys(&blank_attrs));
    assert!(
        blank_attrs.iter().all(|(key, _)| key != KEY_FAILED_REASON),
        "P0-3：reason 全空白时不得写出 {KEY_FAILED_REASON}（禁止空串/占位），实际属性={blank_attrs:?}"
    );
}

// ── 契约 3：非失败 phase + 无 reason ⇒ 键缺席 ──────────────────────────────

#[test]
fn completed_phase_without_reason_omits_key() {
    // Act
    let attributes = turn_span_attributes(&turn_input("completed", None, None));
    println!("[P0-3c] phase=completed reason=None ⇒ keys={:?}", keys(&attributes));

    // Assert
    assert!(
        attributes.iter().all(|(key, _)| key != KEY_FAILED_REASON),
        "P0-3：completed 且无原因时不得写出 {KEY_FAILED_REASON}，实际属性={attributes:?}"
    );
    assert_eq!(
        attr_str(&attributes, "agent.turn.phase"),
        Some("completed"),
        "兼容锁：agent.turn.phase 必须仍按既有映射输出 completed，实际属性={attributes:?}"
    );
}

// ── 契约 4：既有属性集逐字不变（兼容锁） ──────────────────────────────────

#[test]
fn existing_turn_attribute_set_is_unchanged() {
    // Arrange：加字段前的形态（reason=None）与加字段后的形态（reason=Some）各取一份。
    let baseline = turn_span_attributes(&turn_input("failed", Some("user-1"), None));
    let with_reason =
        turn_span_attributes(&turn_input("failed", Some("user-1"), Some("tool_failed: timeout")));
    println!("[P0-3c] baseline keys={:?}", keys(&baseline));
    println!("[P0-3c] with_reason keys={:?}", keys(&with_reason));

    // Assert 1：既有键序逐字不变（user_id 为 Some ⇒ 第 7 个键）。
    let expected_keys: Vec<&str> = EXISTING_KEYS
        .iter()
        .copied()
        .chain(std::iter::once(KEY_USER_ID))
        .collect();
    assert_eq!(
        keys(&baseline),
        expected_keys,
        "兼容锁：既有 turn 属性键序必须逐字不变（新增 failed_reason 不得重排/覆盖既有键）"
    );

    // Assert 2：新增键只**追加**，不改变既有键的相对顺序与取值。
    let mut with_reason_without_new: Vec<&str> = keys(&with_reason);
    with_reason_without_new.retain(|key| *key != KEY_FAILED_REASON);
    assert_eq!(
        with_reason_without_new, expected_keys,
        "兼容锁：去掉新键后，既有属性键序必须与历史逐字一致，实际={with_reason_without_new:?}"
    );
    assert_eq!(
        with_reason
            .iter()
            .filter(|(key, _)| key == KEY_FAILED_REASON)
            .count(),
        1,
        "P0-3：{KEY_FAILED_REASON} 必须恰好出现一次，实际属性={with_reason:?}"
    );

    // Assert 3：既有键的**取值**逐字不变。
    for key in expected_keys {
        assert_eq!(
            attr_str(&with_reason, key),
            attr_str(&baseline, key),
            "兼容锁：既有属性 `{key}` 的取值不得因新增 failed_reason 而改变"
        );
    }
}

// ── 契约 5：失败原因同样必须脱敏 ──────────────────────────────────────────

#[test]
fn failed_reason_is_sanitized() {
    // Arrange：包装形态（Bearer 凭证）+ 家目录路径 —— 二者都在 `ponysentry::sanitize` 脱敏面内。
    // 假值必须落在 `check-secrets` 的安全集（`sk-test` 前缀）；Bearer 规则仍会脱敏 ⇒ 断言语义不变。
    const FAKE_TOKEN: &str = "sk-test-TOPSECRET-9876543210";
    let reason = format!(
        "tool_failed: provider 401 (Authorization: Bearer {FAKE_TOKEN}) at /home/dm/pony-agent/secret.ts"
    );
    let input = turn_input("failed", None, Some(&reason));

    // Act
    let attributes = turn_span_attributes(&input);

    // Assert
    let value = attr_str(&attributes, KEY_FAILED_REASON)
        .unwrap_or_else(|| panic!("P0-3：failed_reason 必须写出，实际属性={attributes:?}"));
    assert!(
        !value.contains(FAKE_TOKEN),
        "P0-3：{KEY_FAILED_REASON} 不得包含未脱敏凭证（原串 `{FAKE_TOKEN}`），实际={value:?}"
    );
    assert!(
        !value.contains("/home/dm"),
        "P0-3：{KEY_FAILED_REASON} 不得包含未脱敏家目录路径（原串 `/home/dm`），实际={value:?}"
    );
    assert!(
        value.contains("[REDACTED_SECRET]"),
        "P0-3：{KEY_FAILED_REASON} 必须出现脱敏标记 `[REDACTED_SECRET]`，实际={value:?}"
    );
    assert!(
        value.contains("[USER_HOME]"),
        "P0-3：{KEY_FAILED_REASON} 必须出现路径脱敏标记 `[USER_HOME]`，实际={value:?}"
    );
    // 归因语义仍可读：前缀保留。
    assert!(
        value.contains("tool_failed"),
        "P0-3：脱敏不得抹掉归因前缀 `tool_failed`，实际={value:?}"
    );
}
