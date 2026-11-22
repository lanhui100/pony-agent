//! 输入框多行导航单测：↑/↓ 跨行移动 + 首尾行边界 + 8 行显示上限。
//!
//! 覆盖：
//! - 键位判据 [`composer_vertical_navigation`]：只认裸 ↑/↓（返回方向），带 Shift/Alt/Ctrl 一律 `None`；
//! - 高度上限 [`composer_input_height`] / [`MAX_COMPOSER_ROWS`]：正文 1~n 行夹在 `[2, 8]`；
//! - 光标目标 [`vertical_cursor_target`]：同列跨行（含软折行 / CJK 宽字符），
//!   首行 ↑ ⇒ 文字头部、尾行 ↓ ⇒ 文字末尾，中间行更短时落到行尾且不横向漂移；
//! - 状态接线 [`TuiState::move_cursor_vertical`]：按上一帧文本区宽度算列，编辑动作清空期望列。

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn up() -> ComposerVerticalDirection {
    ComposerVerticalDirection::Up
}

fn down() -> ComposerVerticalDirection {
    ComposerVerticalDirection::Down
}

#[test]
fn vertical_navigation_only_bare_up_and_down() {
    assert_eq!(
        composer_vertical_navigation(&key(KeyCode::Up, KeyModifiers::NONE)),
        Some(up()),
        "裸 ↑ 必须走输入框内跨行移动"
    );
    assert_eq!(
        composer_vertical_navigation(&key(KeyCode::Down, KeyModifiers::NONE)),
        Some(down()),
        "裸 ↓ 必须走输入框内跨行移动"
    );
    for code in [KeyCode::Up, KeyCode::Down] {
        for modifiers in [
            KeyModifiers::SHIFT,
            KeyModifiers::ALT,
            KeyModifiers::CONTROL,
        ] {
            assert_eq!(
                composer_vertical_navigation(&key(code, modifiers)),
                None,
                "{modifiers:?} + {code:?} 不得抢走既有语义"
            );
        }
    }
    for code in [
        KeyCode::Left,
        KeyCode::Right,
        KeyCode::PageUp,
        KeyCode::PageDown,
        KeyCode::Char('k'),
    ] {
        assert_eq!(
            composer_vertical_navigation(&key(code, KeyModifiers::NONE)),
            None,
            "{code:?} 不是垂直导航键"
        );
    }
}

#[test]
fn composer_height_caps_at_eight_rows() {
    assert_eq!(MAX_COMPOSER_ROWS, 8, "输入框最多显示 8 行");
    assert_eq!(composer_input_height(0), 2, "空输入 = 1 行正文 + 1 行呼吸");
    assert_eq!(composer_input_height(1), 2, "单行 + 呼吸");
    assert_eq!(composer_input_height(6), 7, "上限内逐行长高");
    assert_eq!(composer_input_height(7), 8, "第 7 行正文刚好撑满 8 行");
    assert_eq!(composer_input_height(8), 8, "第 8 行正文进入窗口滚动");
    assert_eq!(
        composer_input_height(10_000),
        8,
        "长正文不得突破 8 行上限"
    );
}

#[test]
fn first_line_up_goes_to_text_head_last_line_down_to_text_end() {
    // 单行：↑ ⇒ 文字头部，↓ ⇒ 文字末尾。
    let text = "hello";
    assert_eq!(
        vertical_cursor_target(text, 3, 80, up(), None),
        (0, Some(3)),
        "首行 ↑ 必须落到文字头部"
    );
    assert_eq!(
        vertical_cursor_target(text, 3, 80, down(), None),
        (5, Some(3)),
        "尾行 ↓ 必须落到文字末尾"
    );

    // 显式换行：第二行 ↑ 回到首行同列；尾行 ↓ 到全文末尾。
    let text = "ab\ncdef";
    assert_eq!(vertical_cursor_target(text, 4, 80, up(), None), (1, Some(1)));
    assert_eq!(
        vertical_cursor_target(text, 4, 80, down(), None),
        (7, Some(1))
    );
    assert_eq!(
        vertical_cursor_target(text, 1, 80, up(), None),
        (0, Some(1)),
        "首行 ↑ 不受列影响：一律到文字头部"
    );
}

#[test]
fn middle_lines_keep_display_column_without_drift() {
    let text = "abcd\nab\nabcdef";
    // 从尾行行尾（列 6）上移到「ab」行：该行更短 ⇒ 落到行尾（换行符位置），但期望列保留 6。
    let (cursor, desired) = vertical_cursor_target(text, 14, 80, up(), None);
    assert_eq!((cursor, desired), (7, Some(6)), "短行落到行尾");

    let (cursor, desired) = vertical_cursor_target(text, cursor, 80, up(), desired);
    assert_eq!((cursor, desired), (4, Some(6)), "继续上移仍按原期望列");

    // 再下移两行：期望列 6 回到长行 ⇒ 光标回到全文末尾。
    let (cursor, desired) = vertical_cursor_target(text, cursor, 80, down(), desired);
    assert_eq!((cursor, desired), (7, Some(6)));
    let (cursor, desired) = vertical_cursor_target(text, cursor, 80, down(), desired);
    assert_eq!((cursor, desired), (14, Some(6)), "回到最右列");
}

#[test]
fn soft_wrap_and_cjk_width_are_respected() {
    // 软折行：宽 3 时 "abcdef" 折成 2 行。
    assert_eq!(
        vertical_cursor_target("abcdef", 1, 3, down(), None),
        (4, Some(1)),
        "下行同列（宽 3 折行）"
    );
    assert_eq!(
        vertical_cursor_target("abcdef", 4, 3, up(), None),
        (1, Some(1))
    );
    // 折行边界上的光标（行 0 行尾）↑ 仍按「首行」处理 ⇒ 文字头部。
    assert_eq!(
        vertical_cursor_target("abcdef", 3, 3, up(), None),
        (0, Some(3))
    );

    // CJK 宽字符：每个汉字 2 列，宽 4 时两字一行。
    assert_eq!(
        vertical_cursor_target("你好世界", 1, 4, down(), None),
        (3, Some(2)),
        "汉字按显示列对齐（你=2 列）"
    );
    assert_eq!(
        vertical_cursor_target("你好世界", 3, 4, up(), None),
        (1, Some(2))
    );
    assert_eq!(
        vertical_cursor_target("你好世界", 0, 4, down(), None),
        (2, Some(0))
    );
}

#[test]
fn empty_text_is_a_no_op() {
    assert_eq!(vertical_cursor_target("", 0, 80, up(), None), (0, Some(0)));
    assert_eq!(vertical_cursor_target("", 0, 80, down(), None), (0, Some(0)));
}

#[test]
fn state_vertical_move_uses_render_width_and_resets_desired_col_on_edit() {
    let mut state = TuiState::default();
    state.last_input_rect = Some(Rect {
        x: 0,
        y: 0,
        width: 80,
        height: 2,
    });
    state.set_input("ab\ncd");
    assert_eq!(state.input_cursor, 5);

    state.move_cursor_vertical(up());
    assert_eq!(
        state.input_cursor, 2,
        "尾行 ↑ 到首行同列（首行只有 2 列 ⇒ 行尾）"
    );
    assert_eq!(state.input_desired_col, Some(2));

    state.move_cursor_vertical(down());
    assert_eq!(state.input_cursor, 5, "首行 ↓ 回到同行同列（尾行行尾）");
    assert_eq!(state.input_desired_col, Some(2));

    // 编辑动作必须清空期望列，否则后续 ↑/↓ 会按陈旧列跳。
    state.insert_char('X');
    assert_eq!(state.input_desired_col, None);
    state.move_cursor_vertical(up());
    assert_eq!(state.input_cursor, 2);
    state.move_cursor_left();
    assert_eq!(state.input_desired_col, None);
}

#[test]
fn empty_composer_vertical_move_keeps_cursor_at_head() {
    let mut state = TuiState::default();
    state.last_input_rect = Some(Rect {
        x: 0,
        y: 0,
        width: 40,
        height: 2,
    });
    state.move_cursor_vertical(up());
    assert_eq!(state.input_cursor, 0);
    state.move_cursor_vertical(down());
    assert_eq!(state.input_cursor, 0, "空输入框 ↓ 不得越界");
}
