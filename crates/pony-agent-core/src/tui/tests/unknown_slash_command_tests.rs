//! 契约 F-D：拼错的斜杠命令必须被拦下（不得作为聊天消息发给模型）。
//!
//! 判定真值源：`tui::unknown_slash_command`（首 token 口径，纯函数）；
//! 提示文案真值源：`tui::unknown_command_notice`。
//!
//! 铁律：每条用例都是真实断言（无 `#[ignore]`、无 skip）；反例与正例同等重要——
//! 「不吞正文」的契约 F-B 反例（`/clear 正文`、`/etc/hosts 是什么`）必须继续放行。

use super::*;

// ── 正例：形如命令词但不在命令表 → 必须被识别为「拼错」 ──────────────────────────

#[test]
fn skill_gesture_is_not_unknown_command() {
    assert_eq!(unknown_slash_command("/skill:review"), None);
    assert_eq!(unknown_slash_command("/skill:review 这个需求怎么写"), None);
}

#[test]
fn unknown_command_detects_typo_of_registered_command() {
    assert_eq!(unknown_slash_command("/cleer"), Some("/cleer"));
    assert_eq!(unknown_slash_command("/thikning"), Some("/thikning"));
    assert_eq!(unknown_slash_command("/clearx"), Some("/clearx"));
    assert_eq!(unknown_slash_command("/cost"), Some("/cost"));
}

#[test]
fn unknown_command_is_case_insensitive_and_trimmed() {
    // 命令表大小写不敏感 ⇒ 拼错判定同样大小写不敏感（`/CLEER` 仍是拼错）。
    assert_eq!(unknown_slash_command("  /CLEER  "), Some("/CLEER"));
    assert_eq!(unknown_slash_command("\t/Help2\n"), Some("/Help2"));
}

#[test]
fn unknown_command_with_arguments_is_still_blocked() {
    // 首 token 未知 + 带正文：仍是拼错的命令（否则 `/cleer 正文` 会被静默发给模型）。
    assert_eq!(
        unknown_slash_command("/cleer 这个需求怎么拆"),
        Some("/cleer")
    );
}

// ── 反例：不是「拼错的命令」→ 必须放行（保持契约 F-B 不吞正文）────────────────────

#[test]
fn registered_commands_are_not_unknown() {
    for (cmd, _) in AVAILABLE_COMMANDS.iter() {
        assert_eq!(
            unknown_slash_command(cmd),
            None,
            "命令表内的 `{cmd}` 不得被判为未知命令"
        );
        assert_eq!(
            unknown_slash_command(&cmd.to_uppercase()),
            None,
            "`{cmd}` 的大写形式不得被判为未知命令"
        );
    }
}

#[test]
fn historic_aliases_are_not_unknown() {
    for alias in HISTORIC_COMMAND_ALIASES.iter() {
        assert_eq!(
            unknown_slash_command(alias),
            None,
            "历史别名 `{alias}` 是命令语义，不得被判为未知命令"
        );
    }
    assert_eq!(unknown_slash_command("/EXIT"), None);
    assert_eq!(unknown_slash_command("/Effort"), None);
}

#[test]
fn command_with_body_keeps_flowing_as_message() {
    // 契约 F-B：已知命令 + 正文 ⇒ 普通消息（不得清空上下文），不属于本拦截面。
    assert_eq!(unknown_slash_command("/clear 这个需求怎么拆"), None);
    assert_eq!(unknown_slash_command("/plan 帮我拆解X"), None);
}

#[test]
fn slash_in_the_middle_of_text_is_not_intercepted() {
    // 拦截面只覆盖「输入开头」的命令位：句中/行中的 `/xxx` 属于正文（举例、引用、路径），
    // 即使拼写不在命令表内也必须照旧发送，不得拦截。
    assert_eq!(unknown_slash_command("请用 /cleer 清理一下上下文"), None);
    assert_eq!(unknown_slash_command("我在想 /thikning 是不是错拼"), None);
    assert_eq!(unknown_slash_command("配置里写的是 /cleer 吧"), None);
    // 多行粘贴：首 token 是正文 ⇒ 后面的 `/cleer` 行仍属正文。
    assert_eq!(unknown_slash_command("第一行说明\n/cleer 第二行"), None);
}

#[test]
fn path_like_input_is_not_a_command() {
    // 路径/表达式（含第二个 `/`）不是命令词 ⇒ 照旧作为普通消息发送。
    assert_eq!(unknown_slash_command("/etc/hosts 是什么"), None);
    assert_eq!(unknown_slash_command("/usr/bin/env"), None);
    assert_eq!(unknown_slash_command("//clear"), None);
}

#[test]
fn bare_slash_and_plain_text_are_not_unknown_commands() {
    assert_eq!(unknown_slash_command("/"), None);
    assert_eq!(unknown_slash_command("/ "), None);
    assert_eq!(unknown_slash_command(""), None);
    assert_eq!(unknown_slash_command("   "), None);
    assert_eq!(unknown_slash_command("hello"), None);
    assert_eq!(unknown_slash_command("clear"), None);
}

// ── 提示文案：必须点名命令并指向 /help ─────────────────────────────────────────────

#[test]
fn unknown_command_notice_names_token_and_points_to_help() {
    let notice = unknown_command_notice("/cleer");
    assert!(
        notice.contains("/cleer"),
        "提示必须点名被拦下的命令，实际 = {notice:?}"
    );
    assert!(
        notice.contains("/help"),
        "提示必须给出自助入口 `/help`，实际 = {notice:?}"
    );
    assert!(
        notice.contains("已拦截"),
        "提示必须说明「已拦截未发送」，实际 = {notice:?}"
    );
}
