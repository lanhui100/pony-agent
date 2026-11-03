//! wave-60 C9 行为级黑盒验收（独立测试者，红相先写）：TUI 停止呈现必须**事件驱动**，
//! 禁止按键处理里本地伪造终态。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §C9。
//! 判据：
//! - 流式中双击 Esc ⇒ 发请求后状态行"正在停止…"（`is_streaming` 仍为 true，**不是**终态）；
//! - 收到 `turn:cancelled` ⇒ 状态行"已停止"、`is_streaming == false`；
//! - **未收到终态事件时不得**打印"已中断本轮生成。"（防假终态）；
//! - 流式中 Ctrl+C 单按**保留**立即中断语义（不要求双击），但同样走"请求→事件驱动终态"。
//!
//! 测试缝（Lead 裁决，T5 必须交付）：
//! `TuiState::apply_stop_requested(&mut self, turn_id: &str)` /
//! `TuiState::apply_stop_terminal(&mut self, turn_id: &str, stop_reason: Option<&str>)`
//! —— 纯状态迁移，可测；本文件以 bin-as-module 方式 include `src/bin/tui.rs`（与既有
//! `tests/tui_acceptance.rs` 同法），**不修改** `src/bin/tui.rs`。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use tui::{CtrlCAction, MessageRole, TuiState};

const FAKE_TERMINAL_MESSAGE: &str = "已中断本轮生成。";

fn streaming_state() -> TuiState {
    let mut state = TuiState::default();
    state.is_streaming = true;
    state.add_user_message("stop me");
    state.append_assistant_chunk("partial answer");
    state
}

/// C9：停止请求后只进入"正在停止…"（仍处流式态，非终态）。
#[test]
fn c9_stop_requested_enters_stopping_without_faking_terminal() {
    // Arrange
    let mut state = streaming_state();

    // Act
    state.apply_stop_requested("turn-c9-request");

    // Assert
    assert!(
        state.is_streaming,
        "C9：发出停止请求后**不得**本地伪造终态（is_streaming 必须仍为 true，终态只能由事件驱动）"
    );
    assert!(
        state.status_text.contains("正在停止"),
        "C9：停止请求后状态行必须是「正在停止…」，实际 = {:?}",
        state.status_text
    );
    assert!(
        !state
            .messages
            .iter()
            .any(|message| message.content.contains(FAKE_TERMINAL_MESSAGE)),
        "C9：未收到终态事件前不得打印「{FAKE_TERMINAL_MESSAGE}」"
    );
}

/// C9：收到终态事件后状态行"已停止"、退出流式态，且不打印假终态文案。
#[test]
fn c9_stop_terminal_event_drives_stopped_state() {
    // Arrange
    let mut state = streaming_state();
    state.apply_stop_requested("turn-c9-terminal");

    // Act
    state.apply_stop_terminal("turn-c9-terminal", Some("user"));

    // Assert
    assert!(
        !state.is_streaming,
        "C9：收到 turn:cancelled 后必须退出流式态"
    );
    assert!(
        state.status_text.contains("已停止"),
        "C9：收到终态事件后状态行必须是「已停止」，实际 = {:?}",
        state.status_text
    );
    assert!(
        !state
            .messages
            .iter()
            .any(|message| message.content.contains(FAKE_TERMINAL_MESSAGE)),
        "C9：终态呈现必须由事件驱动，不得打印本地伪造的「{FAKE_TERMINAL_MESSAGE}」"
    );
    let assistant = state
        .messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::Assistant)
        .expect("C9：前置条件——必须存在流式助手消息");
    assert_eq!(
        assistant.content, "partial answer",
        "C9：终态替换不得吞掉已流式正文"
    );
}

/// C9：流式中 Ctrl+C 单按保留「中断本轮」语义（不要求双击），且不得走本地伪造终态。
#[test]
fn c9_ctrl_c_single_press_keeps_interrupt_semantics() {
    // Arrange / Act
    let verdict_streaming = tui::ctrl_c_action(true, false);
    let verdict_with_draft = tui::ctrl_c_action(false, true);
    let verdict_idle = tui::ctrl_c_action(false, false);

    // Assert
    assert_eq!(
        verdict_streaming,
        CtrlCAction::InterruptTurn,
        "C9：流式中 Ctrl+C 单按必须仍为「中断本轮」（本波不得改成双击）"
    );
    assert_eq!(
        verdict_with_draft,
        CtrlCAction::ClearDraft,
        "C9 非目标：非流式且有草稿时 Ctrl+C 仍是清草稿"
    );
    assert_eq!(
        verdict_idle,
        CtrlCAction::Exit,
        "C9 非目标：非流式且无草稿时 Ctrl+C 仍是退出"
    );
}
