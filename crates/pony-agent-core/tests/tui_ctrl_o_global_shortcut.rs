//! 独立验收：`Ctrl+O`「隐藏 Agent 过程」升级为**全局快捷键**。
//!
//! 契约真值源：`.dev-team/contract-matrix-tui-process-fold-scroll-settings.md` C12（修订 R2）。
//!
//! 断言分层（全部确定性、无 sleep、无真实终端）：
//! - 纯函数层：`is_hide_process_shortcut` 真值表（仅精确 `Ctrl+O` 命中）；
//! - 源码顺序护栏（机械可查的承诺）：`InputKey` 分发表里 `Ctrl+O` 的消费点必须**早于**
//!   所有「吸收所有按键」的模态分支 —— 否则设置面板 / 选择器 / 审批 / 问答打开时会吞键，
//!   这正是本次修订要修复的缺陷；
//! - 状态机层：`/settings` 面板自身对 `Char('o')` 是 no-op（证明只有前置全局分支能让它
//!   在面板内生效），且快捷开关与面板行读写同一 `hide_process` 字段。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use crossterm::event::{KeyCode, KeyModifiers};

const SOURCE: &str = include_str!("../src/bin/tui.rs");

#[test]
fn ctrl_o_predicate_only_matches_exact_control_o() {
    assert!(tui::is_hide_process_shortcut(
        KeyCode::Char('o'),
        KeyModifiers::CONTROL
    ));
    // 裸 o / 大写 / 其它修饰键组合都不应触发（避免与终端复制等组合键冲突）。
    assert!(!tui::is_hide_process_shortcut(
        KeyCode::Char('o'),
        KeyModifiers::NONE
    ));
    assert!(!tui::is_hide_process_shortcut(
        KeyCode::Char('O'),
        KeyModifiers::CONTROL
    ));
    assert!(!tui::is_hide_process_shortcut(
        KeyCode::Char('o'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT
    ));
    assert!(!tui::is_hide_process_shortcut(
        KeyCode::Char('o'),
        KeyModifiers::CONTROL | KeyModifiers::ALT
    ));
    assert!(!tui::is_hide_process_shortcut(
        KeyCode::Char('p'),
        KeyModifiers::CONTROL
    ));
    assert!(!tui::is_hide_process_shortcut(KeyCode::F(2), KeyModifiers::NONE));
}

/// 返回 `needle` 在 `src` 中首次出现且位于 `after` 之后的位置。
fn first_after(src: &str, needle: &str, after: usize) -> Option<usize> {
    src[after..].find(needle).map(|i| i + after)
}

#[test]
fn global_ctrl_o_handler_runs_before_every_modal_branch() {
    // 机械护栏（契约 C12 修订 R2）：Ctrl+O 的消费点必须早于所有「吸收所有按键」的模态分支。
    // 若有人把它移回面板快捷键块（模态分支之后），此测试即失败。
    let input_key = SOURCE
        .find("UiEvent::InputKey(key) => {")
        .expect("InputKey 分发表必须存在");
    let handler = first_after(
        SOURCE,
        "if is_hide_process_shortcut(key.code, key.modifiers)",
        input_key,
    )
    .expect("InputKey 分发表里必须存在前置的 Ctrl+O 全局处理分支");

    // 所有会 `continue` 吞掉字符键的模态分支锚点（按事件循环中的原样字符串）。
    let modal_branches = [
        "if state.mode == InteractionMode::SelectingSession {",
        "if state.mode == InteractionMode::SelectingProvider {",
        "if state.mode == InteractionMode::SelectingModel {",
        "if state.mode == InteractionMode::SelectingReasoningEffort {",
        "if state.mode == InteractionMode::Settings {",
        "if state.mode == InteractionMode::AddingProvider {",
        "if state.mode == InteractionMode::PermissionApproval {",
        "if state.mode == InteractionMode::AskUserQuestion {",
        "if state.mode == InteractionMode::PlanMode && plan_mode_consumes_key(&key) {",
        "if state.mode == InteractionMode::AgentRoster {",
        "if state.mode == InteractionMode::TaskBoard {",
    ];
    for modal in modal_branches {
        let pos = first_after(SOURCE, modal, input_key)
            .unwrap_or_else(|| panic!("事件循环缺少模态分支锚点: {modal}"));
        assert!(
            handler < pos,
            "Ctrl+O 全局分支（偏移 {handler}）必须早于模态分支 `{modal}`（偏移 {pos}），\
             否则面板/选择器打开时会被吞键"
        );
    }
}

#[test]
fn settings_panel_does_not_swallow_ctrl_o_char() {
    // 面板自身收到 Char('o') 是 no-op：它不会切换 hide_process。
    // 因此「面板打开时 Ctrl+O 仍生效」唯一的实现路径就是前置全局分支。
    let mut state = tui::TuiState::default();
    state.mode = tui::InteractionMode::Settings;
    let before = state.hide_process;
    assert_eq!(state.handle_settings_key(KeyCode::Char('o')), None);
    assert_eq!(
        state.hide_process, before,
        "Settings handle_settings_key 不得因 Char('o') 改动 hide_process"
    );
}

#[test]
fn shortcut_and_panel_share_single_hide_process_field() {
    // C12：快捷开关（toggle_hide_process）与面板行必须读写同一字段。
    let mut state = tui::TuiState::default();
    assert!(!state.hide_process, "Default 保持过程可见基线");
    let after = state.toggle_hide_process();
    assert!(after && state.hide_process, "切换后字段必须为 true");
    assert_eq!(
        state.settings_rows()[0].2, state.hide_process,
        "面板行状态必须与字段同源"
    );
}
