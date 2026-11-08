//! 回归护栏：`Ctrl+O` 切换「隐藏 Agent 过程」后，消息区必须**当帧**贴底。
//!
//! 缺陷现象（用户报告）：历史较长时按 `Ctrl+O` 展开 Agent 过程，画面从顶部逐帧滚动
//! 到底部（肉眼可见的「播放滚动动画」），而不是打开即在底部。
//!
//! 根因：过程可见性翻转会让历史整体重排（`max_scroll_offset` 骤增），而
//! `auto_scroll` 的阻尼跟随每帧最多推进 3 行；开关本身不重置偏移，于是从旧底部
//! 一路逐帧追赶新底部。
//!
//! 口径：内容整体重排属于**非增量**变化，不应复用增量阻尼；只要仍处于自动贴底
//! 跟踪，就必须一次吸附到新底部。用户手动上翻（`auto_scroll == false`）时不强制拉回。
//!
//! 范式来源：`tests/tui_process_fold_acceptance.rs`（`#[path = "../src/bin/tui.rs"]`
//! + `TestBackend` + `row_text`/`all_text`）。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

fn screen(state: &mut tui::TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("harness：TestBackend 必须可创建");
    terminal
        .draw(|f| tui::ui(f, state))
        .expect("harness：ui() 渲染不得失败");
    terminal.backend().buffer().clone()
}

/// 多帧渲染直到自动贴底收敛（阻尼跟随单帧最多推进 3 行）。
fn screen_until_settled(state: &mut tui::TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("harness：TestBackend 必须可创建");
    let mut last = terminal.backend().buffer().clone();
    for _ in 0..400 {
        terminal
            .draw(|f| tui::ui(f, state))
            .expect("harness：ui() 渲染不得失败");
        last = terminal.backend().buffer().clone();
        if state.scroll_offset >= state.max_scroll_offset {
            break;
        }
    }
    last
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

fn msg(role: tui::MessageRole, content: &str) -> tui::ChatMessage {
    tui::ChatMessage {
        role,
        content: content.to_string(),
        timestamp_ms: 0,
    }
}

/// 20 轮「思考 + 工具 + 回答」的长会话：隐藏过程时已溢出视口，
/// 展开后行数翻倍以上（保证阻尼跟随不可能一帧到达底部）。
fn long_session(hide_process: bool) -> tui::TuiState {
    let mut state = tui::TuiState::default();
    state.set_hide_process(hide_process);
    for i in 0..20 {
        state.add_user_message(&format!(
            "第{i:02}问：这是一段足够长的用户消息，用来把消息区撑到溢出视口，保证贴底时最后一条内容仍然完整可见。"
        ));
        state.messages.push(msg(
            tui::MessageRole::Thinking,
            &format!("第{i:02}段思考：先核对工作区改动\n再看分支差异\n最后给出结论"),
        ));
        state.add_tool_event(true, "shell · git status --short");
        state.messages.push(msg(
            tui::MessageRole::Assistant,
            &format!("第{i:02}段回答：工作区干净，没有需要提交的改动。"),
        ));
    }
    state
}

/// 展开过程后**单帧**必须已经在底部（不得逐帧滚动追赶）。
#[test]
fn showing_process_snaps_to_bottom_in_one_frame() {
    let mut state = long_session(true);
    let settled = all_text(&screen_until_settled(&mut state, 100, 30));
    assert!(
        squeeze(&settled).contains(&squeeze("第19段回答")),
        "harness：隐藏态收敛后最后一条内容必须可见，实际屏幕 = {settled}"
    );
    assert!(state.auto_scroll, "harness：隐藏态收敛后必须处于自动贴底");

    let before = state.max_scroll_offset;
    state.toggle_hide_process();
    let one = all_text(&screen(&mut state, 100, 30));

    assert!(
        state.max_scroll_offset > before,
        "harness：展开过程后内容必须变长（before={before}, after={}）",
        state.max_scroll_offset
    );
    assert_eq!(
        state.scroll_offset, state.max_scroll_offset,
        "展开过程后第一帧就必须贴底：offset={}, max={}",
        state.scroll_offset, state.max_scroll_offset
    );
    assert!(
        squeeze(&one).contains(&squeeze("第19段回答")),
        "展开过程后第一帧必须已显示最后一条内容，实际屏幕 = {one}"
    );
}

/// 收起过程（内容变短）同理：单帧贴底，不出现「回滚动画」。
#[test]
fn hiding_process_snaps_to_bottom_in_one_frame() {
    let mut state = long_session(false);
    let _ = screen_until_settled(&mut state, 100, 30);
    assert!(state.auto_scroll, "harness：收敛后必须处于自动贴底");

    state.toggle_hide_process();
    let one = all_text(&screen(&mut state, 100, 30));
    assert_eq!(
        state.scroll_offset, state.max_scroll_offset,
        "收起过程后第一帧就必须贴底：offset={}, max={}",
        state.scroll_offset, state.max_scroll_offset
    );
    assert!(
        squeeze(&one).contains(&squeeze("第19段回答")),
        "收起过程后第一帧必须已显示最后一条内容，实际屏幕 = {one}"
    );
}

/// 用户手动上翻阅读历史时，开关不得强行把视口拉回底部。
#[test]
fn toggling_process_keeps_user_scroll_when_not_following() {
    let mut state = long_session(true);
    let _ = screen_until_settled(&mut state, 100, 30);
    state.scroll_up(6);
    let _ = screen(&mut state, 100, 30);
    assert!(!state.auto_scroll, "harness：上翻后必须退出自动贴底");
    let offset_before = state.scroll_offset;

    state.toggle_hide_process();
    let _ = screen(&mut state, 100, 30);
    assert!(
        !state.auto_scroll && state.scroll_offset <= offset_before,
        "手动上翻态下开关不得强行贴底：before={offset_before}, after={}",
        state.scroll_offset
    );
}
