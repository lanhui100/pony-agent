//! wave-63 T3 红相：stopping 期间按键**不得静默吞**（`TuiState::note_stopping_key` 纯迁移）。
//!
//! 判据源（Lead 本轮定义）：
//! - `TuiState` 新增纯迁移 `note_stopping_key(&mut self) -> bool`：
//!   - `stopping_turn_id.is_some()` ⇒ 状态提示置为「已请求停止，等待安全收口…」并返回 `false`
//!     （表示**不**重复派发停止请求）；
//!   - 否则返回 `true`（调用方按原逻辑处理），**不改文案**。
//!
//! 写法沿用既有 bin-as-module 约定（同 `tests/tui_stop_wave60.rs` / `tests/tui_acceptance.rs`）：
//! `#[path = "../src/bin/tui.rs"] mod tui;`，不改 `src/**`。
//!
//! 红相预期：当前 `TuiState` 尚无 `note_stopping_key` ⇒ 编译期 `no method named` （函数不存在）。
//! 该文件不使用 `#[ignore]`、不吞异常、不做空断言。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use tui::TuiState;

/// stopping 期间的状态提示必须包含这句（可读、可断言、非静默）。
const STOPPING_KEY_HINT: &str = "等待安全收口";
/// 非 stopping 基线的状态文案（用于断言「不得被本次调用改写」）。
const IDLE_STATUS_TEXT: &str = "Ready";

fn stopping_state() -> TuiState {
    let mut state = TuiState::default();
    state.is_streaming = true;
    state.add_user_message("stop me");
    state.append_assistant_chunk("partial answer");
    state.apply_stop_requested("turn-63-stopping");
    state
}

/// T3-1：stopping 中按键 ⇒ 返回 false（不重复派发停止请求）+ 状态提示给出「等待安全收口」，
/// 且停止目标与流式态均保持不变（终态只能由事件驱动）。
#[test]
fn t3_stopping_key_returns_false_and_reports_waiting_for_settlement() {
    // Arrange
    let mut state = stopping_state();
    let stopping_before = state.stopping_turn_id.clone();
    let status_before = state.status_text.clone();

    // Act
    let should_handle_normally = state.note_stopping_key();

    // Assert：① 不得重复派发停止请求
    assert!(
        !should_handle_normally,
        "T3：stopping 期间的按键必须返回 false（表示不重复派发停止请求），实际 true"
    );
    // Assert：② 状态提示不得静默，必须给出「等待安全收口」
    assert!(
        state.status_text.contains(STOPPING_KEY_HINT),
        "T3：stopping 期间按键后状态文案必须含「{STOPPING_KEY_HINT}」，实际 = {:?}（按键前 {:?}）",
        state.status_text,
        status_before
    );
    // Assert：③ 停止目标不变（不得被按键改写/清空）
    assert_eq!(
        state.stopping_turn_id, stopping_before,
        "T3：stopping_turn_id 必须保持不变（按键不得改写停止目标）"
    );
    assert!(
        state.stopping_turn_id.is_some(),
        "T3：stopping 期间不得因按键提前清空停止目标（终态须由 turn:cancelled 事件驱动）"
    );
    // Assert：④ 仍在流式态（不得本地伪造终态）
    assert!(
        state.is_streaming,
        "T3：stopping 期间按键不得本地伪造终态（is_streaming 必须仍为 true）"
    );
}

/// T3-2：非 stopping 时 ⇒ 返回 true（调用方按原逻辑处理），且**不得**改写状态文案。
#[test]
fn t3_non_stopping_key_is_not_intercepted_and_keeps_status_text() {
    // Arrange：无在飞停止请求（未进入 stopping）
    let mut state = TuiState::default();
    state.status_text = IDLE_STATUS_TEXT.to_string();
    assert!(
        state.stopping_turn_id.is_none(),
        "T3：前置条件——本例必须不在 stopping 态"
    );

    // Act
    let should_handle_normally = state.note_stopping_key();

    // Assert
    assert!(
        should_handle_normally,
        "T3：非 stopping 态必须返回 true（不拦截按键，调用方按原逻辑处理）"
    );
    assert_eq!(
        state.status_text, IDLE_STATUS_TEXT,
        "T3：非 stopping 态不得被本次调用改写状态文案"
    );
    assert!(
        !state.status_text.contains(STOPPING_KEY_HINT),
        "T3：非 stopping 态不得出现「{STOPPING_KEY_HINT}」提示（避免误报停止中）"
    );
}

/// T3-3：连续按键（stopping 期间）必须保持幂等：始终 false、提示恒在、停止目标恒定。
#[test]
fn t3_repeated_stopping_keys_stay_idempotent() {
    // Arrange
    let mut state = stopping_state();
    let stopping_before = state.stopping_turn_id.clone();

    // Act
    let first = state.note_stopping_key();
    let second = state.note_stopping_key();

    // Assert
    assert!(!first && !second, "T3：stopping 期间每次按键都必须返回 false（幂等）");
    assert!(
        state.status_text.contains(STOPPING_KEY_HINT),
        "T3：重复按键后提示必须仍然可见，实际 = {:?}",
        state.status_text
    );
    assert_eq!(
        state.stopping_turn_id, stopping_before,
        "T3：重复按键不得改写停止目标"
    );
    assert!(state.is_streaming, "T3：重复按键不得伪造终态");
}
