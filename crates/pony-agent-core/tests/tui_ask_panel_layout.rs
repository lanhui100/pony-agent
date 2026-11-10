//! wave-ask-panel-multiselect：TUI Ask 内联面板的**渲染级**版式契约。
//!
//! 用户可读要求（本文件把它变成可机器断言的判据）：
//! 1. 面板有上下内边距（不是内容贴边）；
//! 2. 问题正文独占一行（不与「提问」chip 挤在同一行）；
//! 3. 问题与选项之间有间距行；
//! 4. 选项长文本折行，绝不越过面板右缘、绝不省略号截断；
//! 5. 多选问题渲染勾选框（`▢`/`▣`），单选仍用圆点（`○`/`●`）。
//!
//! 渲染接缝沿用 `tests/tui_overlay_cancel_acceptance.rs` 的同法：`#[path] mod tui` +
//! ratatui `TestBackend`，断言落在字符缓冲上（黑盒，不碰私有状态）。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use tui::{AskUserQuestion, QuestionOption, TuiState};

const QUESTION_SENTINEL: &str = "PANELQ_选择本次交付的执行架构";

fn render(state: &mut TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("create test backend");
    terminal
        .draw(|frame| tui::ui(frame, state))
        .expect("render frame");
    terminal.backend().buffer().clone()
}

fn row_text(buf: &Buffer, y: u16) -> String {
    let mut out = String::new();
    for x in 0..buf.area.width {
        out.push_str(buf[(x, y)].symbol());
    }
    out
}

/// CJK 宽字符在 `Buffer` 里占 2 个 cell（第二格是空白占位）；内容包含判断前先去空白。
fn squeeze(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn rows(buf: &Buffer) -> Vec<String> {
    (0..buf.area.height).map(|y| row_text(buf, y)).collect()
}

fn find_row(buf: &Buffer, needle: &str) -> Option<u16> {
    (0..buf.area.height).find(|&y| squeeze(&row_text(buf, y)).contains(&squeeze(needle)))
}

/// 与 `ui()` 同一口径：`composer_top = height - 4`（色带 + 指标行 + 底面行）。
fn composer_top(height: u16) -> u16 {
    height - 4
}

fn state_with(question: AskUserQuestion) -> TuiState {
    let mut state = TuiState::default();
    state.add_user_message("上一轮的历史消息");
    state.append_assistant_chunk("历史回答正文");
    state.start_ask_user("req-layout".to_string(), vec![question]);
    state
}

fn question(multi_select: bool, options: Vec<QuestionOption>) -> AskUserQuestion {
    AskUserQuestion {
        id: "q-layout".to_string(),
        header: Some("验收分支".to_string()),
        question: QUESTION_SENTINEL.to_string(),
        multi_select,
        options,
    }
}

fn two_options() -> Vec<QuestionOption> {
    vec![
        QuestionOption {
            label: "分支 A".to_string(),
            description: Some("先做冒烟".to_string()),
        },
        QuestionOption {
            label: "分支 B".to_string(),
            description: Some("直接全量".to_string()),
        },
    ]
}

#[test]
fn ask_panel_has_top_bottom_padding_and_question_on_its_own_line() {
    let mut state = state_with(question(false, two_options()));
    let buf = render(&mut state, 100, 34);
    let composer = composer_top(34);

    // 标题行：`◆ 提问 [分组] i/n`（不再携带问题正文）。
    let title_row = find_row(&buf, "提问").expect("面板必须渲染标题行");
    assert!(
        !squeeze(&row_text(&buf, title_row)).contains(&squeeze(QUESTION_SENTINEL)),
        "问题正文必须独立成行，不得与「提问」chip 同行：{:?}",
        row_text(&buf, title_row)
    );

    // 上内边距：标题行上面一行是空白，再上面才是发丝线。
    assert!(
        row_text(&buf, title_row - 1).trim().is_empty(),
        "标题行之上必须有 1 行上内边距，实际 = {:?}",
        row_text(&buf, title_row - 1)
    );
    assert!(
        row_text(&buf, title_row - 2).contains('▁'),
        "上内边距之上必须是面板发丝线，实际 = {:?}",
        row_text(&buf, title_row - 2)
    );

    // 问题独占一行，且其后一个间距行、再之后是第一个选项。
    let question_row = find_row(&buf, QUESTION_SENTINEL).expect("面板必须渲染问题正文");
    assert!(question_row > title_row, "问题正文必须在标题之下");
    assert!(
        row_text(&buf, question_row + 1).trim().is_empty(),
        "问题与选项之间必须有 1 行间距，实际 = {:?}",
        row_text(&buf, question_row + 1)
    );
    assert!(
        squeeze(&row_text(&buf, question_row + 2)).contains("分支A"),
        "间距行之后必须紧跟第一个选项，实际 = {:?}",
        row_text(&buf, question_row + 2)
    );

    // 下内边距：最后一条内容行（键位提示）之下一行整行空白，再之下才是 Composer。
    let hint_row = (0..composer)
        .rev()
        .find(|&y| row_text(&buf, y).contains("esc"))
        .expect("面板必须渲染键位提示（含 esc 出口）");
    assert_eq!(
        hint_row,
        composer - 2,
        "键位提示必须在 Composer 之上第 2 行（下内边距占 1 行），实际 row{hint_row}"
    );
    assert!(
        row_text(&buf, hint_row + 1).trim().is_empty(),
        "面板下内边距必须是整行空白，实际 = {:?}",
        row_text(&buf, hint_row + 1)
    );
}

#[test]
fn ask_panel_long_option_wraps_inside_panel_without_ellipsis() {
    let long_label = "OPTLABEL_这是一个非常长的选项标签用于验证折行不会越过面板右缘并且不丢字";
    let long_description = "说明文字同样很长，折行后必须继续与标签列对齐而不是溢出到面板之外";
    let options = vec![QuestionOption {
        label: long_label.to_string(),
        description: Some(long_description.to_string()),
    }];
    let mut state = state_with(question(false, options));
    let width = 60u16;
    let buf = render(&mut state, width, 30);
    let composer = composer_top(30);

    let title_row = find_row(&buf, "提问").expect("标题行");
    let hint_row = (0..composer)
        .rev()
        .find(|&y| row_text(&buf, y).contains("esc"))
        .expect("键位提示");
    let panel_text: String = (title_row..hint_row)
        .map(|y| squeeze(&row_text(&buf, y)))
        .collect();

    assert!(
        panel_text.contains(&squeeze(long_label)),
        "长选项必须完整折行显示（不得丢字），面板文本 = {panel_text:?}"
    );
    // 「自定义输入…」是未聚焦占位文案里的省略号，不属于选项截断；先剔除再判。
    let option_text = panel_text.replace("自定义输入…", "");
    assert!(
        !option_text.contains('…'),
        "选项禁止省略号截断，面板文本 = {panel_text:?}"
    );
    // 行宽不越界：折行后的每一行（含缩进与右留白）都不得超过面板宽度。
    for y in title_row..=hint_row {
        let used = row_text(&buf, y).chars().count();
        assert!(
            used <= width as usize,
            "row{y} 宽度 {used} 超过面板宽度 {width}"
        );
    }
}

#[test]
fn ask_panel_multi_select_renders_checkboxes_and_single_select_keeps_radios() {
    // 多选：两个未勾选方框，不得出现单选圆点。
    let mut multi = state_with(question(true, two_options()));
    let multi_buf = render(&mut multi, 100, 34);
    let multi_rows = rows(&multi_buf);
    let unchecked: usize = multi_rows.iter().map(|row| row.matches('▢').count()).sum();
    let checked: usize = multi_rows.iter().map(|row| row.matches('▣').count()).sum();
    let radios: usize = multi_rows.iter().map(|row| row.matches('○').count()).sum();
    assert_eq!(unchecked, 2, "多选问题必须为每个选项渲染未勾选框");
    assert_eq!(checked, 0, "初始状态不得有已勾选项");
    assert_eq!(radios, 0, "多选问题不得渲染单选圆点");

    // 勾选第一项：方框转为已勾，且只影响该项。
    if let Some(ask) = multi.ask_user_state.as_mut() {
        ask.toggle_current_option();
    }
    let toggled_buf = render(&mut multi, 100, 34);
    let toggled_rows = rows(&toggled_buf);
    let toggled_checked: usize = toggled_rows.iter().map(|row| row.matches('▣').count()).sum();
    let toggled_unchecked: usize = toggled_rows.iter().map(|row| row.matches('▢').count()).sum();
    assert_eq!(toggled_checked, 1, "勾选后必须出现 1 个已勾方框");
    assert_eq!(toggled_unchecked, 1, "未勾选项必须保持方框");

    // 单选：圆点语义逐字不变（焦点 `●` + 其余 `○`）。
    let mut single = state_with(question(false, two_options()));
    let single_buf = render(&mut single, 100, 34);
    let single_rows = rows(&single_buf);
    let active: usize = single_rows.iter().map(|row| row.matches('●').count()).sum();
    let idle: usize = single_rows.iter().map(|row| row.matches('○').count()).sum();
    let boxes: usize = single_rows
        .iter()
        .map(|row| row.matches('▢').count() + row.matches('▣').count())
        .sum();
    assert_eq!(active, 1, "单选焦点项必须渲染 ●");
    assert_eq!(idle, 1, "单选其余项必须渲染 ○");
    assert_eq!(boxes, 0, "单选问题不得渲染勾选框");
}
