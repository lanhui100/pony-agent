//! TUI 三态权限模式（只读 / 工作区写入 / 完全访问）验收测试。
//!
//! 覆盖：
//! 1. /settings 面板包含 default_permission_mode 项，支持三态循环切换与状态文案提示；
//! 2. /permission 快捷指令解析及直接传参切换（/permission full-access 等）；
//! 3. Ctrl+P 全局快捷键判定与轮转；
//! 4. 状态行右槽时分轮播（模型名称与权限模式按 3s 间隔平滑交替展示）；
//! 5. build_turn_input 携带正确的 tool_authorization_mode 注入内核。

use pony_tui as tui;

use crossterm::event::{KeyCode, KeyModifiers};
use pony_agent_core::agent::headless::PermissionMode;

#[test]
fn test_default_permission_mode_is_workspace_write() {
    let state = tui::TuiState::default();
    assert_eq!(state.permission_mode, PermissionMode::WorkspaceWrite);
    assert_eq!(state.permission_mode_label(), "工作区写入");
    assert_eq!(state.permission_mode.tool_authorization_mode(), "workspace_write");
}

#[test]
fn test_rotate_permission_mode_cycles_through_all_three_modes() {
    let mut state = tui::TuiState::default();
    assert_eq!(state.permission_mode, PermissionMode::WorkspaceWrite);

    // WorkspaceWrite -> FullAccess
    let mode1 = state.rotate_permission_mode();
    assert_eq!(mode1, PermissionMode::FullAccess);
    assert_eq!(state.permission_mode, PermissionMode::FullAccess);
    assert_eq!(state.permission_mode_label(), "完全访问");
    assert_eq!(state.permission_mode.tool_authorization_mode(), "full_access");

    // FullAccess -> ReadOnly
    let mode2 = state.rotate_permission_mode();
    assert_eq!(mode2, PermissionMode::ReadOnly);
    assert_eq!(state.permission_mode, PermissionMode::ReadOnly);
    assert_eq!(state.permission_mode_label(), "只读");
    assert_eq!(state.permission_mode.tool_authorization_mode(), "read_only");

    // ReadOnly -> WorkspaceWrite
    let mode3 = state.rotate_permission_mode();
    assert_eq!(mode3, PermissionMode::WorkspaceWrite);
    assert_eq!(state.permission_mode, PermissionMode::WorkspaceWrite);
}

#[test]
fn test_settings_panel_permission_row_toggle() {
    let mut state = tui::TuiState::default();
    state.open_settings();
    // 行序真源是 `TuiState::settings_rows()`，按 key 定位避免新增开关后下标漂移。
    let rows = state.settings_rows();
    let permission_row = rows
        .iter()
        .position(|row| row.0 == "default_permission_mode")
        .expect("设置面板必须含 default_permission_mode 行");
    assert_eq!(rows[permission_row].1, "默认权限模式");

    // 选中权限模式项
    state.settings_selected_index = permission_row;

    // 触发切换（WorkspaceWrite -> FullAccess）
    let res = state.handle_settings_key(KeyCode::Enter);
    assert_eq!(res, Some(("default_permission_mode", true)));
    assert_eq!(state.permission_mode, PermissionMode::FullAccess);
    assert!(
        state
            .status_notice
            .as_ref()
            .map(|(t, _)| t.contains("默认权限模式已切换为: 完全访问"))
            .unwrap_or(false)
    );

    // 再次触发切换（FullAccess -> ReadOnly）
    let res = state.handle_settings_key(KeyCode::Char(' '));
    assert_eq!(res, Some(("default_permission_mode", false)));
    assert_eq!(state.permission_mode, PermissionMode::ReadOnly);
    assert!(
        state
            .status_notice
            .as_ref()
            .map(|(t, _)| t.contains("默认权限模式已切换为: 只读"))
            .unwrap_or(false)
    );

    // 再次触发切换（ReadOnly -> WorkspaceWrite）
    let res = state.handle_settings_key(KeyCode::Char(' '));
    assert_eq!(res, Some(("default_permission_mode", true)));
    assert_eq!(state.permission_mode, PermissionMode::WorkspaceWrite);
}

#[test]
fn test_permission_shortcut_predicate() {
    assert!(tui::is_permission_mode_shortcut(KeyCode::Char('p'), KeyModifiers::CONTROL));
    assert!(!tui::is_permission_mode_shortcut(KeyCode::Char('p'), KeyModifiers::NONE));
    assert!(!tui::is_permission_mode_shortcut(KeyCode::Char('P'), KeyModifiers::CONTROL));
    assert!(!tui::is_permission_mode_shortcut(KeyCode::Char('o'), KeyModifiers::CONTROL));
}

#[test]
fn test_status_line_shows_model_and_permission_in_one_frame() {
    // 2026-11-17：状态条取消模型/权限时分轮播（ADR
    // `2026-11-17-tui-task-visibility-bottom-drawer`）——两段必须同帧可见。
    let mut state = tui::TuiState::default();
    state.model = "deepseek-v4".to_string();
    state.permission_mode = PermissionMode::WorkspaceWrite;

    let render = |state: &tui::TuiState| -> String {
        tui::status_bar_line(state, 100)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    };
    let text = render(&state);
    assert!(text.contains("deepseek-v4"), "状态条必须含模型身份，实际：{text:?}");
    assert!(
        text.contains("权限 · 工作区"),
        "状态条必须含权限级别，实际：{text:?}"
    );

    state.permission_mode = PermissionMode::ReadOnly;
    assert!(
        render(&state).contains("权限 · 只读"),
        "权限级别切换必须立刻反映"
    );
    state.permission_mode = PermissionMode::FullAccess;
    assert!(
        render(&state).contains("权限 · 完全访问"),
        "权限级别切换必须立刻反映"
    );
}

#[test]
fn test_build_turn_input_carries_permission_mode() {
    let mut state = tui::TuiState::default();
    state.permission_mode = PermissionMode::ReadOnly;
    let input = tui::build_turn_input(
        &state,
        "test message".to_string(),
        "provider-1".to_string(),
        "model-1".to_string(),
    );
    assert_eq!(input.tool_authorization_mode.as_deref(), Some("read_only"));

    state.permission_mode = PermissionMode::FullAccess;
    let input_fa = tui::build_turn_input(
        &state,
        "test message".to_string(),
        "provider-1".to_string(),
        "model-1".to_string(),
    );
    assert_eq!(input_fa.tool_authorization_mode.as_deref(), Some("full_access"));
}
