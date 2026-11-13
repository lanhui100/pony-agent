//! AT3：脏 DSML 正文注入（未闭合块 / 多块 / 半角变体 / 参数残片）。
//!
//! 判据：
//! 1. 协议片段绝不出现在**持久化正文**（`persisted_assistant_text`）、展示文本
//!    （`display_assistant_text`）、承载文本与下一轮请求体里；
//! 2. 脏块之外的已交付正文与工具结果必须保留（不得因清洗误伤事实）；
//! 3. 旧库既有脏行（占位符 + 参数残片）只读兼容、不再放大。

use pony_agent_core::agent::session::interrupted_turn::{
    display_assistant_text, interrupted_turn_assistant_text, interrupted_turn_transcript_closure,
    persisted_assistant_text, ClosedToolStep, PendingToolCall,
};

use crate::{
    assert_roundtrips_closed, closed, interrupted_history, native_provider, request_body, snapshot_of,
    DELIVERED_TEXT, INTERRUPTED_USER, NEXT_USER,
};

const PROTOCOL_MARKERS: [&str; 6] = [
    "|DSML|",
    "｜DSML｜｜",
    "invoke name=",
    "parameter name=",
    "calls>",
    "［工具调用］",
];

const DIRTY_DELIVERED: &str = "正文甲：先看端口。\n<｜｜DSML｜｜ calls>\n<｜｜DSML｜｜ invoke name=\"Run\">\n<｜｜DSML｜｜ parameter name=\"command\">rm -rf /tmp/dirty</｜｜DSML｜｜ parameter>\n</｜｜DSML｜｜ invoke>\n</｜｜DSML｜｜ calls>\n正文乙：然后复查。\n<||DSML|| calls>\n<||DSML|| invoke name=\"Write\">\nEOF_UNTERMINATED_BLOCK";

fn dirty_closed_steps() -> Vec<ClosedToolStep> {
    vec![
        closed("Read", "DIRTY_RESULT_A 端口 8080 空闲"),
        closed("Bash", "DIRTY_RESULT_B kill 已完成"),
    ]
}

#[test]
fn at3_dirty_dsml_never_reaches_persisted_body_display_or_next_request() {
    // Arrange
    let closed_steps = dirty_closed_steps();
    let pending: Vec<PendingToolCall> = Vec::new();

    // Act
    let assistant_text = interrupted_turn_assistant_text(DIRTY_DELIVERED, &closed_steps, &pending);
    let persisted = persisted_assistant_text(&assistant_text);
    let display = display_assistant_text(&assistant_text);
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DIRTY_DELIVERED, &closed_steps, &pending)
        .expect("AT3：已闭合步骤非空 ⇒ 脏正文注入下仍必须产出闭合片段");
    let serialized_closure = serde_json::to_string(&closure).expect("AT3：闭合片段必须可序列化");
    let session = snapshot_of(
        "at3-dirty",
        interrupted_history(&assistant_text),
        closure.clone(),
    );
    let body = request_body(&native_provider(), &session, NEXT_USER);

    // Assert 1：四个出口都不得泄漏协议片段
    for marker in PROTOCOL_MARKERS {
        assert!(
            !persisted.contains(marker),
            "AT3：持久化正文不得含协议片段 {marker:?}，实际 = {persisted:?}"
        );
        assert!(
            !display.contains(marker),
            "AT3：展示文本不得含协议片段 {marker:?}，实际 = {display:?}"
        );
        assert!(
            !serialized_closure.contains(marker),
            "AT3：闭合片段不得含协议片段 {marker:?}，实际 = {serialized_closure}"
        );
        assert!(
            !assistant_text.contains(marker),
            "AT3：承载文本不得含协议片段 {marker:?}，实际 = {assistant_text:?}"
        );
        if marker != "［工具调用］" {
            assert!(
                !body.contains(marker),
                "AT3：下一轮请求不得含协议片段 {marker:?}，实际请求体 = {body}"
            );
        }
    }

    // Assert 2：已交付正文与工具结果必须保留（清洗不得误伤事实）
    assert!(
        persisted.contains("正文甲：先看端口。"),
        "AT3：协议块之前的已交付正文必须保留，实际 = {persisted:?}"
    );
    assert!(
        persisted.contains("正文乙：然后复查。"),
        "AT3：协议块之后的已交付正文必须保留，实际 = {persisted:?}"
    );
    assert!(
        assistant_text.contains(&closed_steps[0].result_summary)
            && assistant_text.contains(&closed_steps[1].result_summary),
        "AT3：工具结果必须保留（清洗只针对协议片段），实际 = {assistant_text:?}"
    );
    assert_roundtrips_closed(&closure, "AT3");
    assert!(
        body.contains(&closed_steps[1].result_summary),
        "AT3：下一轮请求必须携带工具结果哨兵，实际请求体 = {body}"
    );

    // Assert 3：旧库既有脏行只读兼容（占位符与参数残片不得被放大）
    let legacy_dirty = "［工具调用］［工具调用］\nparameter name=\"command\">legacy-cmd\n旧正文";
    let legacy_persisted = persisted_assistant_text(legacy_dirty);
    assert!(
        !legacy_persisted.contains("parameter name="),
        "AT3：旧脏行的参数残片必须被丢弃，实际 = {legacy_persisted:?}"
    );
    assert!(
        legacy_persisted.matches("［工具调用］").count() <= 1,
        "AT3：旧脏行占位符不得被放大（≤1），实际 = {legacy_persisted:?}"
    );
    assert!(
        legacy_persisted.contains("旧正文"),
        "AT3：旧脏行里的真实正文必须保留，实际 = {legacy_persisted:?}"
    );

    // Assert 4：干净正文逐字保真（不得因清洗误伤）
    let clean = DELIVERED_TEXT;
    assert_eq!(
        persisted_assistant_text(clean),
        clean,
        "AT3：干净正文必须逐字保真（清洗只针对协议块）"
    );
}
