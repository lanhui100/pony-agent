//! 原 `tui.rs` 内联测试模块 `mod transient_error_terminal_tests`（Step1 机械外移，断言零改动）。

use super::*;

/// 真实 trace 里的 45s 超时错误正文（来自 provider `operation timed out` 分类路径）。
const TIMEOUT_ERROR: &str =
    "followup_sync:attempt=1/5 failed class=TransientRetryable timeout after 45001ms";

#[test]
fn transient_summary_no_longer_claims_retry_is_running() {
    let summary = user_facing_error_summary(TIMEOUT_ERROR);
    assert_eq!(
        summary, ERROR_SUMMARY_TRANSIENT,
        "瞬态错误必须收敛到受控终态摘要，实际 = {summary:?}"
    );
    assert!(
        !summary.contains("正在"),
        "StreamFailed 是重试预算终止后的终态，文案不得自称「正在重试」，实际 = {summary:?}"
    );
    assert!(
        summary.contains("重试已用尽"),
        "终态文案必须如实说明重试已结束，实际 = {summary:?}"
    );
}

#[test]
fn transient_summary_renders_as_bold_danger() {
    let severity = system_message_severity(ERROR_SUMMARY_TRANSIENT);
    assert_eq!(
        severity,
        SystemMessageSeverity::Error,
        "受控错误摘要必须判为 Error（旧关键词判定全不命中 ⇒ 曾被渲染成灰色 Info）"
    );
    let style = severity_style(severity, muted());
    assert_eq!(
        style.fg,
        Some(theme::DANGER),
        "终态错误必须用 DANGER 前景色"
    );
    assert!(
        style.add_modifier.contains(Modifier::BOLD),
        "终态错误必须加粗，与普通 System 提示区分"
    );
}

#[test]
fn severity_covers_auth_and_generic_summaries() {
    for summary in [ERROR_SUMMARY_GENERIC, ERROR_SUMMARY_AUTH] {
        assert_eq!(
            system_message_severity(summary),
            SystemMessageSeverity::Error,
            "全部受控错误摘要都必须判为 Error：{summary:?}"
        );
    }
    assert_eq!(
        system_message_severity("已切换提供商 · workbuddy"),
        SystemMessageSeverity::Info,
        "普通提示不得被判为错误（避免满屏红）"
    );
    assert_eq!(
        system_message_severity(ASK_STALE_NOTICE),
        SystemMessageSeverity::Warning,
        "失效类提示应为 Warning 而非 Error"
    );
}

#[test]
fn system_message_render_uses_danger_style_for_terminal_error() {
    let message = ChatMessage {
        role: MessageRole::System,
        content: ERROR_SUMMARY_TRANSIENT.to_string(),
        timestamp_ms: 0,
    };
    let lines = message_lines_for(&message, 80);
    let styled = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .find(|span| span.content.contains("上游服务"))
        .expect("终态错误正文必须被渲染");
    assert_eq!(
        styled.style.fg,
        Some(theme::DANGER),
        "消息区终态错误必须醒目着色，实际 = {:?}",
        styled.style
    );
}
