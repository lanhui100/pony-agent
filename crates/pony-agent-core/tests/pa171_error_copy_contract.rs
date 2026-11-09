//! PA-171 **C5 红相**验收：quota / 限流失败的 TUI 文案必须如实（禁止折叠为「重试已用尽」）。
//!
//! 契约真值源：`management/task-system/03_TASKS/PA-171-goal-round-failure-resilience.md` C5。
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。
//!
//! 缺陷面（已取证，`bin/tui/errors.rs:76/147`）：`user_facing_error_summary` 只要命中
//! `is_transient_provider_error`（429 命中）就统一返回
//! `ERROR_SUMMARY_TRANSIENT = "上游服务暂时不可用，重试已用尽。"` ⇒ 配额耗尽被伪装成
//! "上游抖动 + 重试已用尽"，用户看不到 `5-hour usage limit` / 恢复时间，只剩满屏红字。
//!
//! 断言口径：
//! 1. quota / 限流文本的用户可见摘要**不得**等于「上游服务暂时不可用，重试已用尽。」；
//! 2. 摘要必须给出真实原因（配额/限额/限流/quota）或恢复窗口（恢复/重置/`06:01`）；
//! 3. 反向钉子：auth 与未知错误的既有映射不得被本卡改动（防止"全改成配额文案"式假修复）。
//!
//! 复用 `tests/tui_batch2_regression.rs` 已确立的 bin-as-module 模式：`src/bin/tui.rs` 是
//! `[[bin]]` 目标，只有以模块方式引入同一份真实源码才能在编译期断言其纯函数。
//! 本文件只调纯函数，不渲染、不联网、不读全局 provider。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use tui::{
    user_facing_error_summary, ERROR_SUMMARY_AUTH, ERROR_SUMMARY_GENERIC, ERROR_SUMMARY_TRANSIENT,
};

/// 诊断会话 10:28:51 的真实响应正文形态（Kimi 5 小时限额）。
const QUOTA_ERROR: &str = "provider 返回错误状态：429 Too Many Requests；耗时=18ms；响应正文：{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit, resets at 06:01 UTC\"}}";

/// 需要"如实文案"的错误文本：quota 与限流两类（历史都被折叠成「重试已用尽」）。
const TRUTHFUL_SUMMARY_TEXTS: [(&str, &str); 4] = [
    (QUOTA_ERROR, "Kimi 配额耗尽（429 + exceeded_current_quota_error）"),
    (
        "{\"error\":{\"message\":\"You've reached your 5-hour usage limit\"}}",
        "5 小时用量上限（无 429 状态码的纯文案）",
    ),
    (
        "Request denied: the model is out of its allowance for this billing window",
        "额度耗尽（out of its allowance）",
    ),
    (
        "Error: too many requests, please retry later",
        "纯限流措辞（too many requests）",
    ),
];

/// 摘要是否给出了"真实原因或恢复窗口"（任一命中即可，不锁具体措辞）。
fn mentions_real_cause(summary: &str) -> bool {
    let lower = summary.to_lowercase();
    summary.contains("配额")
        || summary.contains("限额")
        || summary.contains("限流")
        || summary.contains("额度")
        || summary.contains("恢复")
        || summary.contains("重置")
        || summary.contains("06:01")
        || lower.contains("quota")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
}

// ── 断言 1：禁折叠为「重试已用尽」 ────────────────────────────────────────────────

/// C5-a：quota / 限流失败的用户可见摘要不得是「上游服务暂时不可用，重试已用尽。」
#[test]
fn c5_rate_limited_failures_must_not_be_folded_into_retry_exhausted_copy() {
    for (error, ticket) in TRUTHFUL_SUMMARY_TEXTS {
        // Act
        let summary = user_facing_error_summary(error);
        println!("[PA-171 C5][{ticket}] summary = {summary:?}");

        // Assert
        assert_ne!(
            summary, ERROR_SUMMARY_TRANSIENT,
            "C5：{ticket} 被折叠成「{ERROR_SUMMARY_TRANSIENT}」——用户看不到真实原因（配额/恢复时间），\
             这是本卡要修的观测缺陷"
        );
        assert!(
            mentions_real_cause(&summary),
            "C5：{ticket} 的摘要必须给出真实原因或恢复窗口（配额/限额/限流/恢复/重置），实际 {summary:?}"
        );
    }
}

/// C5-b：摘要必须是**一句话可读文案**，不得把上游响应正文整段塞进消息区。
#[test]
fn c5_summary_stays_a_single_readable_line() {
    // Arrange
    let error = QUOTA_ERROR;

    // Act
    let summary = user_facing_error_summary(error);

    // Assert
    assert!(
        !summary.trim().is_empty(),
        "C5：用户可见摘要不得为空"
    );
    assert!(
        !summary.contains('\n') && !summary.contains('\r'),
        "C5：摘要必须是一行可读文案，实际 {summary:?}"
    );
    assert!(
        !summary.contains("响应正文"),
        "C5：摘要不得直接倾倒上游响应正文（详情走折叠区），实际 {summary:?}"
    );
    assert!(
        summary.chars().count() <= 200,
        "C5：摘要长度必须有上界（≤200 字符），实际 {} 字符：{summary:?}",
        summary.chars().count()
    );
}

// ── 断言 2：反向钉子——既有映射不得被打翻 ─────────────────────────────────────────

/// C5-c：auth 与"未知错误"的既有文案保持不变（防止把一切错误都改成配额文案的假修复）。
#[test]
fn c5_auth_and_unknown_copy_are_unchanged() {
    // 401/403 仍归「凭据不可用」。
    for error in [
        "provider 返回错误状态：401 Unauthorized；耗时=4ms",
        "provider 返回错误状态：403 Forbidden；耗时=4ms",
    ] {
        assert_eq!(
            user_facing_error_summary(error),
            ERROR_SUMMARY_AUTH,
            "C5 反向钉子：鉴权失败必须仍归「{ERROR_SUMMARY_AUTH}」，实际文本 = {error}"
        );
    }

    // 与 provider 无关的本地错误仍归通用文案（且不得被误标成配额）。
    let generic = user_facing_error_summary("boom: unexpected local invariant violation");
    assert_eq!(
        generic, ERROR_SUMMARY_GENERIC,
        "C5 反向钉子：非 provider 错误必须仍归「{ERROR_SUMMARY_GENERIC}」"
    );
    assert!(
        !mentions_real_cause(&generic),
        "C5 反向钉子：通用文案不得被误标成配额/限流（假修复指纹），实际 {generic:?}"
    );
}
