//! 「撤回」覆盖层单测（ADR 0113）：双击 Esc 上膛、两段式按键裁决、菜单文案归约。
//!
//! 只测纯逻辑与状态机；真实会话截断/文件还原由
//! `crates/pony-agent-core/tests/tui_rewind_acceptance.rs` 覆盖。

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pony_agent_core::agent::control_plane::rewind_commands::{RewindMode, RewindPointView};

fn point(node_id: &str, turn_index: usize, user_message: &str, files_tracked: usize) -> RewindPointView {
    RewindPointView {
        node_id: node_id.to_string(),
        turn_id: Some(format!("turn-{turn_index}")),
        turn_index,
        title: format!("第 {turn_index} 轮标题"),
        user_message: user_message.to_string(),
        files_tracked,
        created_at_ms: Some(1_000 + turn_index as u64),
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

// ── 双击上膛 ────────────────────────────────────────────────────────────────

#[test]
fn esc_arm_requires_same_session_within_window() {
    let mut arm = rewind::RewindEscArm::default();
    assert!(!arm.press("s1", 1_000, 700), "第一次 Esc 只上膛");
    assert!(
        arm.press("s1", 1_500, 700),
        "同会话 500ms 内的第二次必须开面板"
    );
    // 开面板后清膛：紧接着的第三次不得再判命中
    assert!(!arm.press("s1", 1_550, 700), "命中后必须清膛");
}

#[test]
fn esc_arm_expires_outside_window() {
    let mut arm = rewind::RewindEscArm::default();
    assert!(!arm.press("s1", 1_000, 700));
    assert!(
        !arm.press("s1", 1_800, 700),
        "超过窗口（800ms > 700ms）只重新上膛"
    );
    assert!(arm.press("s1", 1_900, 700), "重新上膛后窗口内仍可命中");
}

#[test]
fn esc_arm_is_session_scoped() {
    let mut arm = rewind::RewindEscArm::default();
    assert!(!arm.press("s1", 1_000, 700));
    assert!(
        !arm.press("s2", 1_100, 700),
        "换会话后第一次 Esc 不得命中（防跨会话误撤回）"
    );
}

#[test]
fn esc_arm_reset_drops_pending_press() {
    let mut arm = rewind::RewindEscArm::default();
    assert!(!arm.press("s1", 1_000, 700));
    arm.reset();
    assert!(!arm.press("s1", 1_100, 700), "reset 后必须重新上膛");
}

// ── 选择推进 ────────────────────────────────────────────────────────────────

#[test]
fn turn_selection_clamps_at_both_ends() {
    let mut state = rewind::RewindState::new(vec![
        point("n2", 2, "第二问", 0),
        point("n1", 1, "第一问", 1),
    ]);
    state.select_prev();
    assert_eq!(state.selected, 0, "已在最新一轮，向上不越界");
    state.select_next();
    assert_eq!(state.selected, 1);
    state.select_next();
    assert_eq!(state.selected, 1, "最后一轮，向下不越界");
    assert_eq!(state.current_point().map(|p| p.node_id.as_str()), Some("n1"));
}

#[test]
fn scope_rows_map_to_modes_and_clamp() {
    assert_eq!(rewind::REWIND_SCOPE_ROWS.len(), 2);
    let mut state = rewind::RewindState::new(vec![point("n1", 1, "问", 0)]);
    assert_eq!(state.scope(), RewindMode::ConversationAndFiles);
    state.select_scope_next();
    assert_eq!(state.scope(), RewindMode::ConversationOnly);
    state.select_scope_next();
    assert_eq!(
        state.scope(),
        RewindMode::ConversationOnly,
        "范围只有两行，向下不越界"
    );
    state.select_scope_prev();
    assert_eq!(state.scope(), RewindMode::ConversationAndFiles);
}

// ── 按键裁决 ────────────────────────────────────────────────────────────────

fn state_with_overlay() -> TuiState {
    let mut state = TuiState::default();
    state.rewind_state = Some(rewind::RewindState::new(vec![
        point("n2", 2, "第二问", 2),
        point("n1", 1, "第一问", 0),
    ]));
    state.mode = InteractionMode::Rewind;
    state
}

#[test]
fn turn_stage_enter_moves_to_scope_and_esc_cancels() {
    let mut state = state_with_overlay();
    assert_eq!(
        rewind::handle_key(&mut state, &key(KeyCode::Enter)),
        rewind::RewindAction::Consumed
    );
    assert_eq!(
        state.rewind_state.as_ref().map(|overlay| overlay.stage),
        Some(rewind::RewindStage::Scope),
        "轮次段 ↵ 进入范围段"
    );
    assert_eq!(
        rewind::handle_key(&mut state, &key(KeyCode::Esc)),
        rewind::RewindAction::Consumed
    );
    assert_eq!(
        state.rewind_state.as_ref().map(|overlay| overlay.stage),
        Some(rewind::RewindStage::Turn),
        "范围段 esc 退回轮次段"
    );
    assert_eq!(
        rewind::handle_key(&mut state, &key(KeyCode::Esc)),
        rewind::RewindAction::Cancel,
        "轮次段 esc 关闭面板"
    );
}

#[test]
fn scope_stage_enter_confirms_with_selected_mode() {
    let mut state = state_with_overlay();
    rewind::handle_key(&mut state, &key(KeyCode::Enter));
    rewind::handle_key(&mut state, &key(KeyCode::Down));
    assert_eq!(
        rewind::handle_key(&mut state, &key(KeyCode::Enter)),
        rewind::RewindAction::Confirm
    );
    assert_eq!(
        state.rewind_state.as_ref().map(|overlay| overlay.scope()),
        Some(RewindMode::ConversationOnly)
    );
}

#[test]
fn scope_stage_number_keys_pick_and_confirm() {
    let mut state = state_with_overlay();
    rewind::handle_key(&mut state, &key(KeyCode::Enter));
    assert_eq!(
        rewind::handle_key(&mut state, &key(KeyCode::Char('2'))),
        rewind::RewindAction::Confirm
    );
    assert_eq!(
        state.rewind_state.as_ref().map(|overlay| overlay.scope()),
        Some(RewindMode::ConversationOnly)
    );
}

#[test]
fn empty_point_list_blocks_entering_scope_stage() {
    let mut state = TuiState::default();
    state.rewind_state = Some(rewind::RewindState::new(Vec::new()));
    state.mode = InteractionMode::Rewind;
    assert_eq!(
        rewind::handle_key(&mut state, &key(KeyCode::Enter)),
        rewind::RewindAction::Consumed
    );
    assert_eq!(
        state.rewind_state.as_ref().map(|overlay| overlay.stage),
        Some(rewind::RewindStage::Turn),
        "没有可撤回轮次时不得进入确认段"
    );
}

#[test]
fn missing_overlay_is_treated_as_cancel() {
    let mut state = TuiState::default();
    assert_eq!(
        rewind::handle_key(&mut state, &key(KeyCode::Enter)),
        rewind::RewindAction::Cancel,
        "面板状态缺失 ⇒ 关闭（不静默吞键）"
    );
}

// ── 菜单文案归约 ────────────────────────────────────────────────────────────

#[test]
fn menu_row_prefers_first_non_empty_message_line() {
    let mut entry = point("n1", 1, "\n  第一行正文  \n第二行不进菜单", 0);
    entry.title = "标题不该被用".to_string();
    assert_eq!(rewind::point_message_summary(&entry), "第一行正文");
}

#[test]
fn menu_row_falls_back_to_title_when_message_empty() {
    let mut entry = point("n1", 1, "   ", 0);
    entry.title = "只有标题".to_string();
    assert_eq!(rewind::point_message_summary(&entry), "只有标题");
    entry.title = "  ".to_string();
    assert_eq!(
        rewind::point_message_summary(&entry),
        "（第 1 轮）",
        "标题也空时给出可辨识占位，不渲染空行"
    );
}
