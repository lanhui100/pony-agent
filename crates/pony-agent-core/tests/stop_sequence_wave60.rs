//! wave-60 C8（Rust 侧）：「双击 Esc」纯判定状态机的黑盒验收（红相先写）。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §C8。
//! 判据表：
//! - 判定窗口 `stop_sequence_ms = 500`（**可注入**）；
//! - 「同一身份」= (`sessionId`, `turnId`, `generation`) 全等；
//! - 第 1 次 press（流式中）仅上膛 ⇒ `false`，不发停止请求；
//! - 窗口内第 2 次同身份 press ⇒ `true`（**恰好一次** stop 请求），随即复位；
//! - 超窗 / 身份变化 / turn 结束（`reset`）⇒ 上膛失效。
//!
//! 本文件不使用 `#[ignore]`，不吞异常：未实现时 `todo!()` 直接判红。

use pony_agent_core::agent::stop_sequence::{StopIdentity, StopSequenceState};

const WINDOW_MS: u64 = 500;

fn identity(session: &str, turn: &str, generation: u64) -> StopIdentity {
    StopIdentity {
        session_id: Some(session.to_string()),
        turn_id: turn.to_string(),
        generation,
    }
}

/// C8：第 1 次按键只上膛，**不**触发停止。
#[test]
fn c8_first_press_only_arms_without_stop_request() {
    // Arrange
    let mut state = StopSequenceState::new(WINDOW_MS);
    let id = identity("session-c8", "turn-c8", 1);

    // Act
    let triggered = state.press(id, 1_000);

    // Assert
    assert!(
        !triggered,
        "C8：第 1 次 Esc（流式中）只上膛，不得返回 true（不得发停止请求）"
    );
}

/// C8：窗口内第 2 次同身份按键 ⇒ 恰好一次触发，且第 3 次重新上膛（不复触发）。
#[test]
fn c8_second_press_within_window_fires_exactly_once() {
    // Arrange
    let mut state = StopSequenceState::new(WINDOW_MS);
    let id = identity("session-c8", "turn-c8", 1);
    state.press(id.clone(), 1_000);

    // Act
    let second = state.press(id.clone(), 1_000 + WINDOW_MS - 1);
    let third = state.press(id, 1_000 + WINDOW_MS);

    // Assert
    assert!(
        second,
        "C8：窗口内第 2 次同身份 Esc 必须触发停止（恰好一次）"
    );
    assert!(
        !third,
        "C8：触发后必须复位；紧接着的第 3 次按键只能是「再次上膛」，不得重复触发"
    );
}

/// C8：超出判定窗口的第 2 次按键无效（重新上膛），窗口内再按才触发。
#[test]
fn c8_press_outside_window_rearms_instead_of_firing() {
    // Arrange
    let mut state = StopSequenceState::new(WINDOW_MS);
    let id = identity("session-c8", "turn-c8", 1);
    state.press(id.clone(), 1_000);

    // Act
    let late = state.press(id.clone(), 1_000 + WINDOW_MS + 1);
    let within_new_window = state.press(id, 1_000 + WINDOW_MS + 2);

    // Assert
    assert!(
        !late,
        "C8：超窗（> {WINDOW_MS}ms）的第 2 次 Esc 不得触发停止，只能重新上膛"
    );
    assert!(
        within_new_window,
        "C8：超窗后重新上膛 ⇒ 紧接着窗口内的下一次 Esc 必须触发"
    );
}

/// C8：身份任一字段变化（session / turn / generation）⇒ 上膛失效。
#[test]
fn c8_identity_change_invalidates_armed_state() {
    let base = identity("session-c8", "turn-c8", 1);

    // case 1: turn 变化
    let mut state = StopSequenceState::new(WINDOW_MS);
    state.press(base.clone(), 1_000);
    assert!(
        !state.press(identity("session-c8", "turn-c8-other", 1), 1_100),
        "C8：turn_id 变化 ⇒ 上一次上膛失效，不得触发"
    );

    // case 2: session 变化
    let mut state = StopSequenceState::new(WINDOW_MS);
    state.press(base.clone(), 1_000);
    assert!(
        !state.press(identity("session-c8-other", "turn-c8", 1), 1_100),
        "C8：session_id 变化 ⇒ 上一次上膛失效，不得触发"
    );

    // case 3: generation 变化
    let mut state = StopSequenceState::new(WINDOW_MS);
    state.press(base.clone(), 1_000);
    assert!(
        !state.press(identity("session-c8", "turn-c8", 2), 1_100),
        "C8：generation 变化 ⇒ 上一次上膛失效，不得触发"
    );

    // case 4: session_id 从 Some 变 None（跨窗口/新会话）
    let mut state = StopSequenceState::new(WINDOW_MS);
    state.press(base, 1_000);
    assert!(
        !state.press(
            StopIdentity {
                session_id: None,
                turn_id: "turn-c8".to_string(),
                generation: 1,
            },
            1_100
        ),
        "C8：session_id 由 Some 变 None 亦属身份变化 ⇒ 不得触发"
    );
}

/// C8：turn 结束 / 会话切换时的 `reset()` 必须清空上膛。
#[test]
fn c8_reset_clears_armed_state() {
    // Arrange
    let mut state = StopSequenceState::new(WINDOW_MS);
    let id = identity("session-c8", "turn-c8", 1);
    state.press(id.clone(), 1_000);

    // Act
    state.reset();
    let after_reset = state.press(id, 1_050);

    // Assert
    assert!(
        !after_reset,
        "C8：reset()（turn 结束 / 会话切换 / 退出流式态）后，下一次按键只能重新上膛"
    );
}

/// C8：判定窗口必须可注入（NFR `stop_sequence_ms`，测试用毫秒级小值）。
#[test]
fn c8_window_is_injectable() {
    // 40ms 窗口：50ms 的间隔已超窗 ⇒ 不触发
    let mut narrow = StopSequenceState::new(40);
    let id = identity("session-c8", "turn-c8", 1);
    narrow.press(id.clone(), 1_000);
    assert!(
        !narrow.press(id.clone(), 1_050),
        "C8：窗口注入为 40ms 时，50ms 后的第 2 次 Esc 必须失效"
    );

    // 5000ms 窗口：50ms 的间隔仍在窗口内 ⇒ 触发
    let mut wide = StopSequenceState::new(5_000);
    wide.press(id.clone(), 1_000);
    assert!(
        wide.press(id, 1_050),
        "C8：窗口注入为 5000ms 时，50ms 后的第 2 次 Esc 必须触发（窗口参数不得被写死）"
    );
}
