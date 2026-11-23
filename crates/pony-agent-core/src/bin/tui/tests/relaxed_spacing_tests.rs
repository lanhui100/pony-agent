//! 宽松行距（`relaxed_spacing`）接线测试。
//!
//! 终端行高不可调，插整行是唯一的「行距」手段；但**全行插行**会让同屏正文行数腰斩。
//! 故最终口径只加厚**块边界**，本测试锁住四条：
//! 1. 默认紧凑：`message_lines_for` 的基线不变（正文行紧邻）；
//! 2. 宽松口径下正文相邻行**仍然紧邻**，且连续正文的总行数与紧凑完全一致
//!    （即「行距只花在段落与标题上」，视口密度不退化）；
//! 3. 段间 2 行、ATX 标题前后各 1 行，且标题后紧跟空行时不得叠成 3 行；
//! 4. 代码块 / 连续列表项不受影响（整块内容不得被拆散）。
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
fn relaxed_keeps_body_lines_adjacent() {
    let texts = plain(&relaxed_lines("第一行\n第二行\n第三行", 80));
    assert_eq!(
        blank_run_between(&texts, "第一行", "第二行"),
        Some(0),
        "宽松行距只在块边界生效，正文行内不得插行: {texts:#?}"
    );
    assert_eq!(
        blank_run_between(&texts, "第二行", "第三行"),
        Some(0),
        "宽松行距只在块边界生效，正文行内不得插行: {texts:#?}"
    );
}

#[test]
fn relaxed_does_not_change_dense_body_line_count() {
    // 核心不变量：连续正文（无空行、无标题）两种口径行数必须相同 ——
    // 宽松只把留白花在段落与标题上，不牺牲同屏可见行数。
    let content = "一\n二\n三\n四\n五";
    let compact = message_lines_for(&message(MessageRole::Assistant, content), 80);
    let relaxed = relaxed_lines(content, 80);
    assert_eq!(
        compact.len(),
        relaxed.len(),
        "连续正文下宽松口径不得增加行数: compact={:#?} relaxed={:#?}",
        plain(&compact),
        plain(&relaxed)
    );
}

#[test]
fn relaxed_paragraph_gap_is_two_blank_lines() {
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
        "宽松口径段落间距加厚到 2 行: {relaxed:#?}"
    );
}

#[test]
fn relaxed_adds_breathing_around_headings() {
    // markdown 渲染会剥掉 `#` 记号，断言按渲染后的文本取值。
    let texts = plain(&relaxed_lines("正文一\n## 小节\n正文二", 80));
    assert_eq!(
        blank_run_between(&texts, "正文一", "小节"),
        Some(1),
        "标题前留 1 行呼吸: {texts:#?}"
    );
    assert_eq!(
        blank_run_between(&texts, "小节", "正文二"),
        Some(1),
        "标题后留 1 行呼吸: {texts:#?}"
    );
    // 紧凑口径不得出现标题呼吸（否则宽松开关会无声地改掉默认观感）。
    let compact = plain(&message_lines_for(
        &message(MessageRole::Assistant, "正文一\n## 小节\n正文二"),
        80,
    ));
    assert_eq!(
        blank_run_between(&compact, "正文一", "小节"),
        Some(0),
        "紧凑口径标题前不留白: {compact:#?}"
    );
    assert_eq!(
        blank_run_between(&compact, "小节", "正文二"),
        Some(0),
        "紧凑口径标题后不留白: {compact:#?}"
    );
}

#[test]
fn relaxed_heading_followed_by_blank_line_does_not_triple() {
    // 标题后已是源空行时，标题呼吸与段间加厚必须收敛到 2 行，不得叠出 3 行。
    let texts = plain(&relaxed_lines("## 小节\n\n正文", 80));
    assert_eq!(
        blank_run_between(&texts, "小节", "正文"),
        Some(2),
        "标题 + 源空行只压成段间 2 行: {texts:#?}"
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
    assert_eq!(code_b, code_a + 1, "代码块行必须保持紧邻");
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
