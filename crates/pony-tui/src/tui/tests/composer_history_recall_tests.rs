//! 壳层历史回填（输入框为空 + Up 回填最近一条用户消息）单测。
//!
//! 覆盖：
//! - 纯判据 [`composer_history_recall`]：仅「空输入框 + 裸 Up」命中；非空 / 带修饰键 / 非 Up 一律不命中；
//! - 状态回填 [`TuiState::recall_last_user_message`]：取最近一条用户消息、光标到末尾、只读不改消息列表；
//! - 无用户消息时不回填（调用方据此回退到原生 Up 滚动）。

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn state_with_messages() -> TuiState {
    let mut state = TuiState::default();
    state.add_user_message("第一条用户消息");
    state.add_user_message("第二条用户消息");
    // 助手回复不应被回填（只取 User 角色）
    state.append_assistant_chunk("助手回复");
    state
}

#[test]
fn recall_judgement_only_empty_bare_up() {
    let up = KeyEvent::new(KeyCode::Up, KeyModifiers::NONE);
    assert!(composer_history_recall(&up, true), "空输入框 + 裸 Up 必须命中");

    // 非空输入框：Up 保持原生（光标移动 / 滚动），不得回填
    assert!(!composer_history_recall(&up, false), "非空输入框不得回填");

    // 修饰键：Shift/Alt/Ctrl + Up 一律不命中
    for modifiers in [
        KeyModifiers::SHIFT,
        KeyModifiers::ALT,
        KeyModifiers::CONTROL,
    ] {
        let key = KeyEvent::new(KeyCode::Up, modifiers);
        assert!(
            !composer_history_recall(&key, true),
            "带 {modifiers:?} 的 Up 不得触发回填"
        );
    }

    // 非 Up 键不命中
    for code in [KeyCode::Down, KeyCode::Char('k'), KeyCode::PageUp] {
        let key = KeyEvent::new(code, KeyModifiers::NONE);
        assert!(!composer_history_recall(&key, true), "{code:?} 不得触发回填");
    }
}

#[test]
fn recall_fills_latest_user_message_and_moves_cursor_to_end() {
    let mut state = state_with_messages();
    state.clear_input();
    assert!(state.input_buffer.is_empty());

    assert!(state.recall_last_user_message(), "存在用户消息时回填应成功");
    assert_eq!(state.input_buffer, "第二条用户消息");
    assert_eq!(
        state.input_cursor,
        state.input_buffer.chars().count(),
        "回填后光标应停在末尾"
    );
    // 回填只读 messages，不得新增/删除消息
    assert_eq!(state.messages.len(), 3);
}

#[test]
fn recall_returns_false_without_user_messages() {
    let mut state = TuiState::default();
    state.append_assistant_chunk("只有助手");
    assert!(!state.recall_last_user_message());
    assert!(state.input_buffer.is_empty(), "无历史时输入框保持为空");
}
