//! P1-3（A · Rust）红相：上传回执结构化。
//!
//! 契约真源：trace 报告 P1-3。
//! `otel::force_export_session_trace` / `control_plane::upload_session_trace` 从「返回 turn 数」
//! 改为返回**结构化回执**：
//!
//! ```ignore
//! pub struct UploadTraceReceipt { pub turns: usize, pub trace_ids: Vec<String>, pub failures: Vec<String> }
//! ```
//!
//! 验收（**不触网**）：
//! 1. 空 trace 列表 ⇒ `turns == 0` / `trace_ids` 空 / `failures` 空，且**不得 Err、不得 panic**；
//! 2. 有 trace 但导出器未初始化 ⇒ 不得静默吞掉失败：要么 `Ok(回执)` 且 `failures` 非空，
//!    要么 `Err(原因)`；二者都必须携带失败原因（本文件按此二选一断言并打印实际分支）；
//! 3. 回执 `Serialize` 后字段名为 camelCase（`turns` / `traceIds` / `failures`），供前端直接消费。
//!
//! 说明：`control_plane::upload_session_trace` 需要完整 ControlPlane 宿主（数据库/会话装配），
//! 集成层构造成本过高；其返回类型与本文件验收的 `UploadTraceReceipt` 是同一个，
//! 序列化契约与「空列表不 Err」语义在 `force_export_session_trace` 侧已覆盖。

use pony_agent_core::agent::otel::{force_export_session_trace, UploadTraceReceipt};
use pony_agent_core::agent::session::TurnTraceRecord;

/// 构造一条最小 trace 记录（其余字段走 `Default`）。
fn trace_record(turn_id: &str, trace_id: Option<&str>) -> TurnTraceRecord {
    TurnTraceRecord {
        turn_id: turn_id.to_string(),
        trace_id: trace_id.map(str::to_string),
        title: format!("Turn {turn_id}"),
        phase: "completed".to_string(),
        ..Default::default()
    }
}

// ── 契约 1：空列表 ⇒ 空回执且不 Err ───────────────────────────────────────

#[test]
fn empty_trace_list_yields_empty_receipt() {
    // Act
    let result = force_export_session_trace("sess-p1-3-empty", &[]);
    println!("[P1-3A] empty traces ⇒ result={result:?}");

    // Assert
    let receipt = result.unwrap_or_else(|error| {
        panic!("P1-3：空 trace 列表必须返回 Ok(空回执)，不得 Err（实际 Err：{error}）")
    });
    assert_eq!(
        receipt.turns, 0,
        "P1-3：空列表回执 turns 必须为 0，实际 {:?}",
        receipt.turns
    );
    assert!(
        receipt.trace_ids.is_empty(),
        "P1-3：空列表回执 trace_ids 必须为空，实际 {:?}",
        receipt.trace_ids
    );
    assert!(
        receipt.failures.is_empty(),
        "P1-3：空列表回执 failures 必须为空，实际 {:?}",
        receipt.failures
    );
}

// ── 契约 2：导出器未初始化 ⇒ 失败原因必须显式（不静默吞） ──────────────────

#[test]
fn uninitialized_exporter_reports_failure_instead_of_silent_loss() {
    // Arrange：本测试二进制内不调用 `otel::init`，导出器必然处于未初始化态。
    let traces = vec![
        trace_record("turn-1", Some("trace-aaaaaaaa1111")),
        trace_record("turn-2", Some("trace-bbbbbbbb2222")),
    ];

    // Act
    let result = force_export_session_trace("sess-p1-3-uninit", &traces);
    println!("[P1-3A] uninitialized exporter ⇒ result={result:?}");

    // Assert：两种合法形态都不得「静默成功」或「静默丢弃」。
    match result {
        Ok(receipt) => {
            assert!(
                !receipt.failures.is_empty(),
                "P1-3：导出器未初始化时 Ok 回执必须列出失败原因（不得假装全部成功），实际 receipts={receipt:?}"
            );
            assert!(
                receipt.trace_ids.len() <= traces.len(),
                "P1-3：回执 trace_ids 数不得多于入参 trace 数（{} > {}）",
                receipt.trace_ids.len(),
                traces.len()
            );
            assert!(
                receipt.turns <= traces.len(),
                "P1-3：回执 turns 不得多于入参 trace 数（{} > {}）",
                receipt.turns,
                traces.len()
            );
            println!(
                "[P1-3A] 分支=Ok(部分回执) turns={} failures={:?}",
                receipt.turns, receipt.failures
            );
        }
        Err(error) => {
            assert!(
                !error.trim().is_empty(),
                "P1-3：Err 必须携带非空失败原因（不得只返回空串）"
            );
            println!("[P1-3A] 分支=Err(原因) error={error}");
        }
    }
}

// ── 契约 3：回执序列化为 camelCase ────────────────────────────────────────

#[test]
fn receipt_serializes_with_camel_case_keys() {
    // Arrange
    let receipt = UploadTraceReceipt {
        turns: 2,
        trace_ids: vec!["aaaaaaaa1111".to_string(), "bbbbbbbb2222".to_string()],
        failures: vec!["chunk 2: timeout".to_string()],
    };

    // Act
    let value = serde_json::to_value(&receipt).expect("P1-3：回执必须可序列化");
    println!("[P1-3A] serialized receipt={value}");

    // Assert
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("P1-3：回执必须序列化为 JSON 对象，实际 {value}"));
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec!["failures", "traceIds", "turns"],
        "P1-3：回执序列化字段名必须为 camelCase（turns/traceIds/failures），实际 {value}"
    );
    assert_eq!(object["turns"], serde_json::json!(2));
    assert_eq!(
        object["traceIds"],
        serde_json::json!(["aaaaaaaa1111", "bbbbbbbb2222"]),
        "P1-3：traceIds 必须按原序序列化"
    );
    assert_eq!(
        object["failures"],
        serde_json::json!(["chunk 2: timeout"]),
        "P1-3：failures 必须原样序列化"
    );
}
