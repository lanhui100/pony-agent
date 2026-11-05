//! W4：`pony resume` CLI 解析与退出提示的验收测试。
//!
//! 缺陷背景（ADR 0062）：TUI 退出后终端停在 kitty 键盘协议/鼠标追踪态，且没有
//! `pony resume <session-id>` 续接入口。本文件只断言 **CLI 面**（纯函数）：
//! - `parse_cli_action` 对 `resume` / `resume <id>` / 非法 `-` 开头的解析；
//! - `resume_hint_line` 的文案单源（必须包含可直接复制的 `pony resume <id>`）。
//!
//! 真实 PTY 的终端还原证据由 `tools/tui-audit/check_exit_resume.py` 承担。
//!
//! 铁律：每条用例都有真实断言；无 `#[ignore]`/skip；不改被测实现。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use tui::CliAction;

#[test]
fn parse_cli_action_without_args_starts_tui() {
    assert!(matches!(tui::parse_cli_action(&[]), CliAction::RunTui));
}

#[test]
fn parse_resume_without_id_targets_latest_session() {
    let args = vec!["resume".to_string()];
    match tui::parse_cli_action(&args) {
        CliAction::ResumeTui(None) => {}
        other => panic!("`resume` 必须解析为 ResumeTui(None)，实际 {other:?}"),
    }
}

#[test]
fn parse_resume_with_explicit_session_id() {
    let args = vec!["resume".to_string(), "session-abcd1234".to_string()];
    match tui::parse_cli_action(&args) {
        CliAction::ResumeTui(Some(id)) => assert_eq!(id, "session-abcd1234"),
        other => panic!("`resume <id>` 必须解析为 ResumeTui(Some(id))，实际 {other:?}"),
    }
}

#[test]
fn parse_resume_with_flag_like_id_is_invalid() {
    let args = vec!["resume".to_string(), "--foo".to_string()];
    assert!(
        matches!(tui::parse_cli_action(&args), CliAction::Invalid(_)),
        "`resume --foo` 不得被当成会话 id（否则用户拼错选项会静默开一个假会话）"
    );
}

#[test]
fn resume_hint_line_is_copy_pastable() {
    let line = tui::resume_hint_line("session-abcd1234");
    assert!(
        line.contains("pony resume session-abcd1234"),
        "退出提示必须包含可直接复制的 `pony resume session-abcd1234`，实际 = {line:?}"
    );
    assert!(
        !line.contains('\n'),
        "退出提示必须是单行（多行会被终端还原序列打散），实际 = {line:?}"
    );
}
