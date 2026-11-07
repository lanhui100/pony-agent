//! 验收：TUI 用户消息块与相邻消息（agent 正文 / 状态消息 / 工具行）之间各 1 行页面底色空白。
//!
//! 真值源：`docs/decisions/0088-tui-user-message-vertical-gap.md`。
//! 几何：用户块 = 上内边距(1) + 正文 + 下内边距(1)，内边距底色为 SURFACE；
//! 本次新增的间距行落在页面底色 CANVAS 上（上方 1 行、下方 1 行），
//! 因此「与相邻消息的行差」由 2 变为 3，且紧邻相邻消息的那一行不是气泡表面。
//!
//! 范式沿用 `tests/tui_thinking_bottom_gap.rs`：`#[path = "../src/bin/tui.rs"] mod tui;`
//! + ratatui `TestBackend` + `row_text` / `find_row` 辅助。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui::Terminal;

/// `theme::CANVAS`（页面底色）字面量：新增间距行必须落在画布上，而不是气泡表面。
const THEME_CANVAS: Color = Color::Rgb(18, 16, 14);
/// `theme::SURFACE`（用户气泡底色）字面量：气泡上下内衬必须仍存在。
const THEME_SURFACE: Color = Color::Rgb(27, 25, 22);

fn msg(role: tui::MessageRole, content: &str) -> tui::ChatMessage {
    tui::ChatMessage {
        role,
        content: content.to_string(),
        timestamp_ms: 0,
    }
}

fn screen_sequence(seq: &[(tui::MessageRole, &str)]) -> Buffer {
    let mut state = tui::TuiState::default();
    for (role, content) in seq {
        state.messages.push(msg(role.clone(), content));
    }
    let mut terminal =
        Terminal::new(TestBackend::new(110, 30)).expect("harness：TestBackend 必须可创建");
    terminal
        .draw(|f| tui::ui(f, &mut state))
        .expect("harness：ui() 渲染不得失败");
    terminal.backend().buffer().clone()
}

fn row_text(buf: &Buffer, y: u16) -> String {
    let mut out = String::new();
    for x in 0..buf.area.width {
        out.push_str(buf[(x, y)].symbol());
    }
    out
}

fn all_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn find_row(buf: &Buffer, needle: &str) -> Option<u16> {
    let needle = squeeze(needle);
    (0..buf.area.height).find(|y| squeeze(&row_text(buf, *y)).contains(&needle))
}

/// 用户块位于这段间距的哪一侧：`After` = `after` 哨兵是用户正文行，
/// `Before` = `before` 哨兵是用户正文行。
#[derive(Clone, Copy)]
enum UserSide {
    After,
    Before,
}

/// 断言两个哨兵行之间**恰好** `expected_rows` 行差，中间行全空白，且这 3 行差里的
/// 两行角色正确：**紧邻相邻消息**的那一行是本次新增的页面底色间距（CANVAS），
/// **紧邻用户正文**的那一行是气泡内衬（SURFACE）。
///
/// 为什么必须区分：用户块自带 1 行上内衬 + 1 行下内衬（同为 SURFACE），
/// 若只校验「中间行全是 CANVAS」会把内衬误判成间距，反之会把间距误判成内衬。
fn assert_sentinel_gap(
    buf: &Buffer,
    before: &str,
    after: &str,
    expected_rows: u16,
    user_side: UserSide,
    why: &str,
) {
    let before_row = find_row(buf, before)
        .unwrap_or_else(|| panic!("{why}：找不到哨兵 {before:?}\n{}", all_text(buf)));
    let after_row = find_row(buf, after)
        .unwrap_or_else(|| panic!("{why}：找不到哨兵 {after:?}\n{}", all_text(buf)));
    assert!(
        after_row > before_row,
        "{why}：后继哨兵必须在下（before={before_row}, after={after_row}）\n{}",
        all_text(buf)
    );
    assert_eq!(
        after_row - before_row,
        expected_rows,
        "{why}：行差必须为 {expected_rows}，实际 before={before_row} → after={after_row}\n{}",
        all_text(buf)
    );
    for y in (before_row + 1)..after_row {
        assert!(
            row_text(buf, y).trim().is_empty(),
            "{why}：间距行 y={y} 必须全空白，实际 = {:?}",
            row_text(buf, y)
        );
    }
    let probe_x = buf.area.width / 2;
    // 远离用户正文的那一行是新增间距（CANVAS），贴用户正文的那一行是气泡内衬（SURFACE）。
    let (canvas_row, surface_row) = match user_side {
        UserSide::After => (before_row + 1, after_row - 1),
        UserSide::Before => (after_row - 1, before_row + 1),
    };
    assert_eq!(
        buf[(probe_x, canvas_row)].bg,
        THEME_CANVAS,
        "{why}：新增间距行 y={canvas_row} 必须落在页面底色 CANVAS 上，实际底色 = {:?}\n{}",
        buf[(probe_x, canvas_row)].bg,
        all_text(buf)
    );
    assert_eq!(
        buf[(probe_x, surface_row)].bg,
        THEME_SURFACE,
        "{why}：紧邻用户正文的 y={surface_row} 必须仍是气泡内衬 SURFACE（间距不得挤掉内衬），\
         实际底色 = {:?}\n{}",
        buf[(probe_x, surface_row)].bg,
        all_text(buf)
    );
}

/// 用户块上方 / 下方与 **agent 正文** 之间各多 1 行空白。
#[test]
fn user_block_keeps_one_blank_row_against_agent_text() {
    let buf = screen_sequence(&[
        (tui::MessageRole::Assistant, "AGENTA"),
        (tui::MessageRole::User, "USERQ"),
        (tui::MessageRole::Assistant, "AGENTB"),
    ]);

    // 上方：agent 正文 → 新增间距 1 → 气泡上内边距 1 → 正文 ⇒ 行差 3。
    assert_sentinel_gap(
        &buf,
        "AGENTA",
        "USERQ",
        3,
        UserSide::After,
        "ADR 0088：agent 正文 → 用户消息 必须多出 1 行页面底色间距（行差 3）",
    );
    // 下方：气泡下内边距 1 → 新增间距 1 → agent 正文 ⇒ 行差 3。
    assert_sentinel_gap(
        &buf,
        "USERQ",
        "AGENTB",
        3,
        UserSide::Before,
        "ADR 0088：用户消息 → agent 正文 必须多出 1 行页面底色间距（行差 3）",
    );
}

/// 用户块与**状态消息**（System）之间同样各 1 行空白。
#[test]
fn user_block_keeps_one_blank_row_against_system_status() {
    let buf = screen_sequence(&[
        (tui::MessageRole::System, "SYSA"),
        (tui::MessageRole::User, "USERQ"),
        (tui::MessageRole::System, "SYSB"),
    ]);

    assert_sentinel_gap(
        &buf,
        "SYSA",
        "USERQ",
        3,
        UserSide::After,
        "ADR 0088：状态消息 → 用户消息 必须多出 1 行页面底色间距（行差 3）",
    );
    assert_sentinel_gap(
        &buf,
        "USERQ",
        "SYSB",
        3,
        UserSide::Before,
        "ADR 0088：用户消息 → 状态消息 必须多出 1 行页面底色间距（行差 3）",
    );
}

/// 用户块与工具行之间同样 1 行空白（工具行只允许间距，不额外加字标）。
#[test]
fn user_block_keeps_one_blank_row_against_tool_rows() {
    let buf = screen_sequence(&[
        (tui::MessageRole::User, "USERQ"),
        (tui::MessageRole::ToolCall, "shell · git status --short"),
    ]);

    assert_sentinel_gap(
        &buf,
        "USERQ",
        "git status --short",
        3,
        UserSide::Before,
        "ADR 0088：用户消息 → 工具行 必须多出 1 行页面底色间距（行差 3）",
    );
}

/// 会话首条用户消息**不得**多出顶部空行：
/// 单条用户消息块 = 上内边距 1 + 正文 1 + 下内边距 1 + 下间距 1 = 4 行。
#[test]
fn first_user_message_has_no_leading_blank_row() {
    let mut state = tui::TuiState::default();
    state.messages.push(msg(tui::MessageRole::User, "USERQ"));
    let mut terminal =
        Terminal::new(TestBackend::new(110, 30)).expect("harness：TestBackend 必须可创建");
    terminal
        .draw(|f| tui::ui(f, &mut state))
        .expect("harness：ui() 渲染不得失败");

    assert_eq!(
        state.render_cache_stats().total_lines,
        4,
        "ADR 0088：首条用户消息块必须是 4 行（上内边距 + 正文 + 下内边距 + 下间距），\
         不得因上方间距多出第 5 行"
    );
}
