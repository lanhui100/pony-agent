//! AT1：超长 + 畸形步骤序列（确定性破坏注入）。
//!
//! 判据：
//! 1. 闭合结构不得因规模/畸形输入被截断成孤儿 tool_calls（补齐优先于截断）；
//! 2. 过程明细逐行有界、单行可展示（换行/NUL 不得切断信息流）；
//! 3. 纯函数输出确定（同入参两次调用逐字相同）。

use pony_agent_core::agent::session::interrupted_turn::{
    interrupted_turn_assistant_text, interrupted_turn_transcript_closure, visible_step_lines,
    ClosedToolStep, PendingToolCall, INTERRUPTED_RESULT_SECTION_HEADER,
};

use crate::{assert_roundtrips_closed, closed, role_of, DELIVERED_TEXT, INTERRUPTED_USER};

const BIG_STEP_COUNT: usize = 500;
const BIG_RESULT_CHARS: usize = 4_000;
const PENDING_COUNT: usize = 50;

fn oversized_closed_steps() -> Vec<ClosedToolStep> {
    let mut steps = Vec::with_capacity(BIG_STEP_COUNT + 5);
    for index in 0..BIG_STEP_COUNT {
        steps.push(closed(
            &format!("Tool_{index:03}"),
            &format!(
                "RESULT_{index:03}_HEAD {}RESULT_{index:03}_TAIL",
                "x".repeat(BIG_RESULT_CHARS)
            ),
        ));
    }
    // 畸形输入：空名 / 换行 / NUL / 8KB 超长名 / 重复名
    steps.push(closed("", "RESULT_EMPTY_NAME"));
    steps.push(closed("Evil\nName", "RESULT_LINE_BREAK"));
    steps.push(closed("Nul\0Name", "RESULT_NUL_BYTE"));
    steps.push(closed(&"L".repeat(8_192), "RESULT_LONG_NAME"));
    steps.push(closed("Tool_000", "RESULT_DUPLICATE_NAME"));
    steps
}

fn oversized_pending_calls() -> Vec<PendingToolCall> {
    let mut pending = Vec::with_capacity(PENDING_COUNT + 1);
    for index in 0..PENDING_COUNT {
        pending.push(PendingToolCall {
            name: format!("Pending_{index:02}"),
        });
    }
    pending.push(PendingToolCall {
        name: String::new(),
    });
    pending
}

#[test]
fn at1_oversized_and_malformed_step_sequence_stays_closed_bounded_and_deterministic() {
    // Arrange
    let closed_steps = oversized_closed_steps();
    let pending = oversized_pending_calls();
    let expected_lines = closed_steps.len() + pending.len();

    // Act（两次同入参调用：确定性判据）
    let assistant_text = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);
    let assistant_text_again = interrupted_turn_assistant_text(DELIVERED_TEXT, &closed_steps, &pending);
    let detail = visible_step_lines(&closed_steps, &pending);
    let detail_again = visible_step_lines(&closed_steps, &pending);
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending)
        .expect("AT1：已闭合步骤非空 ⇒ 超长序列仍必须产出闭合片段（不得因规模退化为 None）");
    let closure_again =
        interrupted_turn_transcript_closure(INTERRUPTED_USER, DELIVERED_TEXT, &closed_steps, &pending)
            .expect("AT1：第二次调用同样必须产出闭合片段");

    // Assert 1：确定性（同入参两次调用逐字相同）
    assert_eq!(
        assistant_text, assistant_text_again,
        "AT1：承载文本必须确定（同入参两次调用逐字相同）"
    );
    assert_eq!(detail, detail_again, "AT1：过程明细必须确定");
    assert_eq!(closure, closure_again, "AT1：闭合片段必须确定");

    // Assert 2：明细逐条可查、逐行单行、总长有界
    assert_eq!(
        detail.len(),
        expected_lines,
        "AT1：过程明细必须逐条覆盖 {expected_lines} 条步骤（含畸形项），实际 {} 条",
        detail.len()
    );
    for (index, line) in detail.iter().enumerate() {
        assert!(
            !line.contains('\n') && !line.contains('\r'),
            "AT1：明细第 {index} 行必须单行（换行不得切断信息流），实际 = {line:?}"
        );
    }
    let total_detail_chars: usize = detail.iter().map(|line| line.chars().count()).sum();
    assert!(
        total_detail_chars < BIG_STEP_COUNT * BIG_RESULT_CHARS * 2,
        "AT1：明细总长必须有界（实际 {total_detail_chars} 字符）"
    );

    // Assert 3：闭合结构不截断成孤儿调用
    assert_roundtrips_closed(&closure, "AT1");
    let serialized = serde_json::to_string(&closure).expect("AT1：闭合片段必须可序列化");
    let tool_messages = closure.iter().filter(|message| role_of(message) == "tool").count();
    assert!(
        tool_messages >= closed_steps.len(),
        "AT1：每个已闭合步骤都必须有结果消息（实际 {tool_messages} < {}）——超长序列不得被静默截断",
        closed_steps.len()
    );
    assert!(
        serialized.contains("RESULT_000_HEAD"),
        "AT1：首个结果内容必须在闭合片段里（不得整体丢弃）"
    );
    assert!(
        serialized.contains("RESULT_DUPLICATE_NAME") && serialized.contains("RESULT_NUL_BYTE"),
        "AT1：畸形/重复名步骤的结果不得被丢弃"
    );

    // Assert 4：承载文本保留结果段标题（结构化过程明细，而非一句"已执行工具步骤"）
    assert!(
        assistant_text.contains(INTERRUPTED_RESULT_SECTION_HEADER),
        "AT1：超长序列的承载文本必须仍含结果段标题 {INTERRUPTED_RESULT_SECTION_HEADER:?}"
    );
    assert!(
        assistant_text.chars().count() < BIG_STEP_COUNT * BIG_RESULT_CHARS * 2,
        "AT1：承载文本总长必须有界，实际 {} 字符",
        assistant_text.chars().count()
    );
}

#[test]
fn at1_empty_and_blank_steps_never_fabricate_success() {
    // Arrange：全空/仅空白的畸形输入
    let blank_steps = vec![closed("", ""), closed("   ", "\t\n")];
    let blank_pending = vec![PendingToolCall {
        name: String::new(),
    }];

    // Act
    let text = interrupted_turn_assistant_text("", &blank_steps, &blank_pending);
    let detail = visible_step_lines(&blank_steps, &blank_pending);
    let closure = interrupted_turn_transcript_closure(INTERRUPTED_USER, "", &blank_steps, &blank_pending);

    // Assert 1：全空白输入不得伪造出"已完成"的事实（契约：三者皆空 ⇒ 空串）
    assert!(
        text.trim().is_empty(),
        "AT1：全空白步骤必须折叠成空串（不得凭空白输入伪造过程清单），实际 = {text:?}"
    );
    // Assert 2：明细不得产出全空行（空步骤必须被折叠，最多保留真实条目数）
    assert!(
        detail.len() <= blank_steps.len() + blank_pending.len(),
        "AT1：明细条数不得超过输入条数，实际 = {detail:?}"
    );
    assert!(
        detail.iter().all(|line| !line.trim().is_empty()),
        "AT1：明细不得产出全空行（空步骤必须被折叠），实际 = {detail:?}"
    );
    if let Some(closure) = closure {
        assert_roundtrips_closed(&closure, "AT1-blank");
    } else {
        // 允许实现选择"无内容 ⇒ None"（契约允许的回退），但必须不是 panic/半截结构
        assert!(
            text.trim().is_empty() || detail.is_empty(),
            "AT1：闭片段为 None 时不得同时产出非空承载文本（二者必须同口径）"
        );
    }
}
