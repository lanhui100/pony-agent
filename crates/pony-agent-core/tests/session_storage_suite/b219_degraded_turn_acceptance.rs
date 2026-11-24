//! B219 降级终态验收（红相）：wire 值 / 派生口径 / 投影检查点 / trace schema 兼容。
//!
//! 现场：一轮 provider follow-up 连续失败（SSE 解码失败 → 同步兜底 connect 超时 →
//! 多次重试超时 → 预算耗尽）后，轮次以「降级收尾」而不是正常完成结束。降级轮必须：
//! 1. 在事件流里可辨识（`turn/end.reason = "degraded"`），且旧读取端遇到未知 wire 值
//!    **不得炸开**，一律退化为 `Completed`（向后兼容读）；
//! 2. 由纯函数从 `fallback_reason` 派生（非空 ⇒ Degraded；None/空串 ⇒ Completed），
//!    避免「降级事实只存在于日志字符串里」；
//! 3. 投影出 assistant 检查点消息（kind = Interrupted，正文带 `[中断轮次检查点`），
//!    否则重启后这一轮在历史里凭空断流；
//! 4. trace 记录带 `providerFallback` 事实，但旧 JSON 无该键仍可反序列化（additive）。
//!
//! 本文件全部为真实断言，不使用真实时钟、不 sleep、不吞异常。

use pony_agent_core::agent::projection::{fold_all, HistoryProjectionState};
use pony_agent_core::agent::session::{HistoryMessageKind, TurnTraceRecord};
use pony_agent_core::agent::turn_event::{completed_turn_end_reason, TurnEndReason, TurnEvent};

/// 1. wire 值契约：`degraded` 双向可达，未知值兼容读为 `Completed`。
#[test]
fn b219_degraded_reason_wire() {
    // Arrange / Act：反序列化 wire 值。
    let degraded: TurnEndReason =
        serde_json::from_str("\"degraded\"").expect("wire 值 \"degraded\" 必须可反序列化");

    // Assert 1
    assert_eq!(
        degraded,
        TurnEndReason::Degraded,
        "wire 值 \"degraded\" 必须映射到 Degraded 变体"
    );

    // Arrange / Act：序列化回 wire。
    let wire = serde_json::to_string(&TurnEndReason::Degraded)
        .expect("Degraded 必须可序列化（落盘事件用它）");

    // Assert 2
    assert_eq!(
        wire, "\"degraded\"",
        "Degraded 的 wire 值必须逐字为 \"degraded\"（serde snake_case）"
    );

    // Arrange / Act：未知 wire 值（旧版本读新日志的反向兼容方向）。
    let unknown: TurnEndReason = serde_json::from_str("\"future_unknown_reason\"")
        .expect("未知 wire 值必须兼容读为 Completed，不得让旧读取端炸开");

    // Assert 3
    assert_eq!(
        unknown,
        TurnEndReason::Completed,
        "未知 wire 值必须退化为 Completed（向后兼容读）"
    );
}

/// 2. 派生口径：`fallback_reason` 非空 ⇒ Interrupted（P0-3）；None / 空串 ⇒ Completed。
#[test]
fn b219_completed_reason_derivation() {
    // Arrange
    let failed_followup = Some("provider_followup_failed: timeout");

    // Act
    let degraded = completed_turn_end_reason(failed_followup);

    // Assert 1（P0-3 修订：兜底不再伪装成 completed+degraded，改用可续跑的 Interrupted）
    assert_eq!(
        degraded,
        TurnEndReason::Interrupted,
        "存在失败兜底原因 ⇒ 本轮必须以 Interrupted 收尾（可重试/可续跑）"
    );

    // Act / Assert 2：空串视为无原因（空白不得伪造降级）。
    assert_eq!(
        completed_turn_end_reason(Some("")),
        TurnEndReason::Completed,
        "空串 fallback_reason 不得判为降级"
    );

    // Act / Assert 3
    assert_eq!(
        completed_turn_end_reason(None),
        TurnEndReason::Completed,
        "无 fallback_reason ⇒ 正常完成"
    );
}

/// 3. 投影契约：`TurnEnd{reason: Degraded}` 必须折出 assistant 检查点消息。
///
/// 事件流严格按验收契约给定：[TurnStart, UserMessage, TurnEnd{Degraded, error:None}]
/// —— 即**没有任何 assistant 正文 / 工具事件**的降级轮。此形态要求投影对 Degraded
/// 无条件合成检查点（正文至少含唯一前缀标记），否则用户在历史里只看到自己的提问。
#[test]
fn b219_degraded_turn_has_checkpoint() {
    // Arrange
    const TURN_ID: &str = "turn-b219-degraded";
    let events: Vec<(u64, TurnEvent)> = vec![
        (
            0,
            TurnEvent::TurnStart {
                turn_id: TURN_ID.to_string(),
            },
        ),
        (
            1,
            TurnEvent::UserMessage {
                turn_id: TURN_ID.to_string(),
                text: "帮我看一下 8080 端口".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            2,
            TurnEvent::TurnEnd {
                turn_id: TURN_ID.to_string(),
                reason: TurnEndReason::Degraded,
                turn_duration_ms: Some(8_400),
                error: None,
                stop_reason: None,
                failure_code: None,
            },
        ),
    ];

    // Act：公开投影 API 折叠。
    let state: HistoryProjectionState =
        fold_all::<HistoryProjectionState, HistoryProjectionState>(&events);
    let messages = state.messages();
    let assistant = messages.iter().find(|message| message.role == "assistant");

    // Assert 1：降级轮必须留下 assistant 检查点（不得只剩一条悬空 user）。
    let assistant = assistant.unwrap_or_else(|| {
        panic!(
            "降级轮必须合成 assistant 检查点消息，实际消息 = {:?}",
            messages
                .iter()
                .map(|message| (message.role.as_str(), message.content.as_str(), message.kind))
                .collect::<Vec<_>>()
        )
    });

    // Assert 2：结构化身份独立成第四态（B274）。
    //
    // 旧口径要求「与中断轮同形（Interrupted）」——但降级轮是**跑完了、降级交付**，
    // 与「没跑完就被终止」是两种不同的续跑决策。共用一个 kind 会让展示层把降级成功
    // 说成「上一轮已中断」。B274 起 `Degraded` 独立成变体，本断言随契约迁移。
    assert_eq!(
        assistant.kind,
        Some(HistoryMessageKind::Degraded),
        "降级轮合成记录的 kind 必须是 Degraded，实际 = {:?}",
        assistant.kind
    );

    // Assert 3：正文带唯一前缀标记（TUI 逐字回放与 is_interrupt_placeholder_text 依赖）。
    assert!(
        assistant.content.contains("[中断轮次检查点"),
        "降级轮检查点正文必须含唯一前缀 `[中断轮次检查点`，实际 = {:?}",
        assistant.content
    );
}

/// 旧版 trace JSON（无 `providerFallback` 键）：其余必填字段与旧写入端同形。
const LEGACY_TRACE_JSON: &str = r#"{
  "turnId": "turn-legacy-b219",
  "title": "旧版降级前 trace",
  "phase": "completed",
  "providerRequestedName": null,
  "providerName": null,
  "providerProtocol": null,
  "providerModel": null,
  "providerSource": null,
  "providerMode": null,
  "buildContextObservation": null,
  "sessionSummary": null,
  "fallbackReason": null,
  "error": null,
  "inputTokens": null,
  "cacheHitInputTokens": null,
  "reasoningTokens": null,
  "outputTokens": null,
  "totalTokens": null,
  "firstTokenLatencyMs": null,
  "turnDurationMs": null
}"#;

/// 4. trace schema 兼容：新增 `providerFallback` 为 additive 字段。
#[test]
fn b219_trace_provider_fallback_schema_compat() {
    // Arrange / Act：旧 JSON（无 providerFallback 键）必须照旧可读。
    let legacy: TurnTraceRecord = serde_json::from_str(LEGACY_TRACE_JSON)
        .expect("旧 JSON 无 providerFallback 键必须仍可反序列化（向后兼容读）");

    // Assert 1：缺键 ⇒ None（旧轮次没有该事实，不得伪造成 false）。
    assert_eq!(
        legacy.provider_fallback, None,
        "旧 JSON 缺 providerFallback 键 ⇒ provider_fallback 必须为 None"
    );
    assert_eq!(
        legacy.turn_id, "turn-legacy-b219",
        "旧 JSON 的既有字段必须解析正确，实际 = {:?}",
        legacy.turn_id
    );

    // Arrange / Act：带 `"providerFallback": true` 的新 JSON。
    let mut with_key: serde_json::Value =
        serde_json::from_str(LEGACY_TRACE_JSON).expect("夹具 JSON 必须可解析为 Value");
    with_key["providerFallback"] = serde_json::Value::Bool(true);
    let parsed: TurnTraceRecord =
        serde_json::from_value(with_key).expect("带 providerFallback 键的 JSON 必须可反序列化");

    // Assert 2
    assert_eq!(
        parsed.provider_fallback,
        Some(true),
        "`providerFallback: true` 必须落到 provider_fallback == Some(true)"
    );

    // Arrange / Act：值为 None 时键必须缺席（不制造噪音字段）。
    let mut record = TurnTraceRecord {
        turn_id: "turn-b219-write".to_string(),
        title: "降级轮写入".to_string(),
        phase: "interrupted".to_string(),
        ..Default::default()
    };
    record.provider_fallback = None;
    let none_json = serde_json::to_value(&record).expect("TurnTraceRecord 必须可序列化");

    // Assert 3
    assert!(
        none_json.get("providerFallback").is_none(),
        "provider_fallback == None 时键必须缺席，实际 JSON 键 = {:?}",
        none_json.as_object().map(|object| object.keys().cloned().collect::<Vec<_>>())
    );

    // Arrange / Act：Some(true) 时必须写出键。
    record.provider_fallback = Some(true);
    let some_json = serde_json::to_value(&record).expect("TurnTraceRecord 必须可序列化");

    // Assert 4
    assert_eq!(
        some_json.get("providerFallback"),
        Some(&serde_json::Value::Bool(true)),
        "provider_fallback == Some(true) 时必须以 camelCase 键写出 true"
    );
}
