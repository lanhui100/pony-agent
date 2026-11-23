//! 宽松行距（`relaxed_spacing`）接线测试。
//!
//! 终端行高不可调，插空行是唯一的「行距」手段。本测试锁住三条口径：
//! 1. 默认紧凑：`message_lines_for` 的基线不变（正文行紧邻）；
//! 2. 开启后正文行之间恰好 1 行呼吸，且**段间距必须大于行距**（否则段落读成连续行）；
//! 3. 代码块 / 连续列表项不受行距影响（整块内容不得被拆散）。
//!
//! 渲染入口用私有的 `message_lines_with_ctx`（同 crate 子模块可直接构造上下文），
//! 保证测的就是 `ui()` 逐帧渲染走的那份实现。

use super::*;

fn message(role: MessageRole, content: &str) -> ChatMessage {
    ChatMessage {
        role,
        content: content.to_string(),
        timestamp_ms: 0,
    }
}

fn plain(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect()
}

fn relaxed_lines(content: &str, width: usize) -> Vec<Line<'static>> {
    message_lines_with_ctx(
        &message(MessageRole::Assistant, content),
        width,
        MessageRenderCtx {
            prev_role: None,
            is_last: false,
            has_preceding_lines: false,
            spinner_tick: 0,
            hide_process: false,
            thinking_collapsed: false,
            relaxed_spacing: true,
        },
    )
}

/// 两个非空白行之间夹着的连续空白行数（找不到则 `None`）。
fn blank_run_between(texts: &[String], first: &str, second: &str) -> Option<usize> {
    let start = texts.iter().position(|text| text.trim() == first)?;
    let end = texts.iter().skip(start + 1).position(|text| text.trim() == second)? + start + 1;
    Some(texts[start + 1..end].iter().filter(|text| text.trim().is_empty()).count())
}

#[test]
fn compact_baseline_keeps_body_lines_adjacent() {
    let lines = message_lines_for(&message(MessageRole::Assistant, "第一行\n第二行"), 80);
    let texts = plain(&lines);
    assert_eq!(
        blank_run_between(&texts, "第一行", "第二行"),
        Some(0),
        "默认紧凑口径下正文行必须紧邻: {texts:#?}"
    );
}

#[test]
fn relaxed_inserts_one_blank_between_body_lines() {
    let lines = relaxed_lines("第一行\n第二行\n第三行", 80);
    let texts = plain(&lines);
    assert_eq!(
        blank_run_between(&texts, "第一行", "第二行"),
        Some(1),
        "宽松行距下相邻正文行之间恰好 1 行呼吸: {texts:#?}"
    );
    assert_eq!(
        blank_run_between(&texts, "第二行", "第三行"),
        Some(1),
        "行距必须逐行生效: {texts:#?}"
    );
}

#[test]
fn relaxed_paragraph_gap_exceeds_line_gap() {
    let compact = plain(&message_lines_for(
        &message(MessageRole::Assistant, "段一\n\n段二"),
        80,
    ));
    let relaxed = plain(&relaxed_lines("段一\n\n段二", 80));
    assert_eq!(
        blank_run_between(&compact, "段一", "段二"),
        Some(1),
        "紧凑口径段落间距 1 行: {compact:#?}"
    );
    assert_eq!(
        blank_run_between(&relaxed, "段一", "段二"),
        Some(2),
        "宽松口径段落间距必须大于行距（2 > 1）: {relaxed:#?}"
    );
}

#[test]
fn relaxed_keeps_code_block_compact() {
    let content = "说明\n```rust\nfn a() {}\nfn b() {}\n```";
    // 语法高亮会把一行拆成多个 span，按整行拼接后的文本匹配行号。
    let texts = plain(&relaxed_lines(content, 80));
    let code_a = texts
        .iter()
        .position(|text| text.contains("fn a()"))
        .expect("代码行 a 必须渲染");
    let code_b = texts
        .iter()
        .position(|text| text.contains("fn b()"))
        .expect("代码行 b 必须渲染");
    assert_eq!(code_b, code_a + 1, "代码块行必须保持紧邻（行距只作用于正文行）");
}

#[test]
fn relaxed_keeps_consecutive_list_items_compact() {
    // markdown 渲染会把 `- ` 规范化成 `· `，断言按渲染后的字形取值。
    let texts = plain(&relaxed_lines("- 甲\n- 乙\n- 丙", 80));
    assert_eq!(
        blank_run_between(&texts, "· 甲", "· 乙"),
        Some(0),
        "连续列表项之间不得插行距: {texts:#?}"
    );
    assert_eq!(
        blank_run_between(&texts, "· 乙", "· 丙"),
        Some(0),
        "连续列表项之间不得插行距: {texts:#?}"
    );
}

#[test]
fn settings_toggle_flips_relaxed_spacing() {
    let mut state = TuiState::default();
    assert!(!state.relaxed_spacing);
    let index = state
        .settings_rows()
        .iter()
        .position(|row| row.0 == "relaxed_spacing")
        .expect("/settings 必须暴露 relaxed_spacing 开关");
    state.settings_selected_index = index;
    let toggled = state.toggle_selected_setting();
    assert_eq!(toggled, Some(("relaxed_spacing", true)));
    assert!(state.relaxed_spacing);
    let toggled_back = state.toggle_selected_setting();
    assert_eq!(toggled_back, Some(("relaxed_spacing", false)));
    assert!(!state.relaxed_spacing);
}
