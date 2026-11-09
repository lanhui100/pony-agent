//! PA-171 **C5 对抗验收（L3-B 补登红相）**：错误文案判定的三个可机判对抗缺陷。
//!
//! 契约真值源：`.dev-team/contract-matrix-pa171.md` §3.1/§3.2（C5 文案、
//! `classify_error_class` / `retry_after` 结构化后缀）+ Lead 的 L3-B 裁决。
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。
//!
//! 三个对抗面（均由 `bin/tui/errors.rs` 的 `auth` 裸 `contains` 与
//! `retry.rs::append_retry_after_marker` 的幂等判定缺陷造成）：
//!
//! **A1 auth 不得吃掉限流**：`user_facing_error_summary` 当前分支序是
//! `contains("401") || contains("403") || contains("api key") || …` **先于**限流分类，
//! 于是 `耗时=4031ms`（含 `403`）、`耗时=401ms`（含 `401`）、正文含 `"api key"` 的 429 配额耗尽
//! 全被折叠成「凭据不可用」——用户被引导去改 API Key，而真因是配额耗尽。
//!
//! **A2 真 auth 仍判 auth**（反向钉子）：修 A1 时不得把 auth 判死；
//! `401 Unauthorized` + `invalid api key` 必须仍映射 `ERROR_SUMMARY_AUTH`。
//!
//! **A3 `retry_after` 标记不得被正文劫持**：`append_retry_after_marker` 以
//! `err.contains("retry_after=")` 做幂等（`retry.rs`），正文里出现同名 token 时
//! **权威头值根本不会写入**；`parse_retry_after_marker` 又取首个出现 ⇒ 取到正文值 999999s
//! （≈11.5 天，冷却窗口被上游正文劫持）。断言：`Retry-After: 60` 的场景下
//! `rate_limit_window_ms` 必须取 60s，不得取正文的 999999。
//!
//! 红相预期（修复前）：A1 / A3 FAIL，A2 PASS（反向钉子的既有行为）。
//! 铁律：真实断言；无 `#[ignore]`/skip；不联网（纯函数 + 构造错误文本）；不改既有 4 个
//! `pa171_*` 冻结文件；不修改 `src/`。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::retry::{
    append_retry_after_marker, rate_limit_recovery_hint, rate_limit_window_ms, RETRY_AFTER_MARKER,
};
use tui::{
    user_facing_error_summary, ERROR_SUMMARY_AUTH, ERROR_SUMMARY_RATE_LIMITED,
    ERROR_SUMMARY_TRANSIENT,
};

/// 摘要是否落在「限流/配额」语义（真实原因），而不是凭据/瞬态/通用文案。
fn mentions_rate_limit_semantics(summary: &str) -> bool {
    let lower = summary.to_lowercase();
    summary.starts_with(ERROR_SUMMARY_RATE_LIMITED)
        || summary.contains("限流")
        || summary.contains("配额")
        || summary.contains("额度")
        || summary.contains("恢复")
        || lower.contains("quota")
        || lower.contains("rate limit")
        || lower.contains("rate_limit")
}

// ── A1：auth 不得吃掉限流 ──────────────────────────────────────────────────────────

/// A1：三个"看起来像 401/403/api key"的 429 配额耗尽文本，都**不得**被判成凭据错误。
///
/// 三个输入都是同一上游限流的真实可达形态：
/// 1. `耗时=4031ms` —— 耗时数字里含 `403`（裸 `contains("403")` 会误判 auth）；
/// 2. `耗时=401ms` —— 耗时数字里含 `401`；
/// 3. 正文含 `"api key"` —— 上游在配额报文里点名 API Key（裸 `contains("api key")` 误判 auth）。
#[test]
fn adversarial_auth_branch_must_not_swallow_rate_limit_errors() {
    const CASES: [(&str, &str); 3] = [
        (
            "provider 返回错误状态：429 Too Many Requests；耗时=4031ms；响应正文：{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit\"}}",
            "耗时 4031ms 内含 403",
        ),
        (
            "provider 返回错误状态：429 Too Many Requests；耗时=401ms；响应正文：{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit\"}}",
            "耗时 401ms 内含 401",
        ),
        (
            "provider 返回错误状态：429 Too Many Requests；耗时=18ms；响应正文：{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit for this api key\"}}",
            "正文点名 api key",
        ),
    ];

    for (error, ticket) in CASES {
        // Act
        let summary = user_facing_error_summary(error);
        println!("[PA-171 A1][{ticket}] summary = {summary:?}");

        // Assert：不得判 auth（否则用户被引导去改 Key，真因是配额耗尽）。
        assert_ne!(
            summary, ERROR_SUMMARY_AUTH,
            "A1：{ticket} 被误判为凭据错误「{ERROR_SUMMARY_AUTH}」——429/配额耗尽被 auth 分支吃掉，\
             用户会去改 API Key 而不是等配额恢复"
        );
        // Assert：必须落在限流/配额语义（真实原因）。
        assert!(
            mentions_rate_limit_semantics(&summary),
            "A1：{ticket} 的摘要必须命中限流/配额文案（含恢复时间或配额原因），实际 {summary:?}"
        );
        // Assert：也不得回退成瞬态通用文案（历史缺陷 F13 的另一个折叠出口）。
        assert_ne!(
            summary, ERROR_SUMMARY_TRANSIENT,
            "A1：{ticket} 不得折叠成「{ERROR_SUMMARY_TRANSIENT}」，实际 {summary:?}"
        );
    }
}

// ── A2：真 auth 仍判 auth（反向钉子） ─────────────────────────────────────────────

/// A2：真凭据失败必须仍映射 `ERROR_SUMMARY_AUTH`（防 A1 的修复把 auth 判死）。
#[test]
fn adversarial_real_auth_error_still_maps_to_auth_copy() {
    // Arrange：真实 401 + 正文点名 invalid api key。
    let error = "provider 返回错误状态：401 Unauthorized；响应正文：{\"error\":{\"message\":\"invalid api key\"}}";

    // Act
    let summary = user_facing_error_summary(error);

    // Assert
    assert_eq!(
        summary, ERROR_SUMMARY_AUTH,
        "A2 反向钉子：真 401 + invalid api key 必须判「{ERROR_SUMMARY_AUTH}」，实际 {summary:?}\
         ——修 A1 时不得把鉴权失败一并判成限流"
    );
    assert!(
        !mentions_rate_limit_semantics(&summary),
        "A2 反向钉子：真鉴权失败不得被标成限流/配额（假修复指纹），实际 {summary:?}"
    );

    // 反向补充：403 + forbidden 同样仍是 auth。
    let forbidden = user_facing_error_summary(
        "provider 返回错误状态：403 Forbidden；响应正文：{\"error\":{\"message\":\"forbidden\"}}",
    );
    assert_eq!(
        forbidden, ERROR_SUMMARY_AUTH,
        "A2 反向钉子：真 403 Forbidden 必须判「{ERROR_SUMMARY_AUTH}」，实际 {forbidden:?}"
    );
}

// ── A3：retry_after 标记不得被正文劫持 ────────────────────────────────────────────

/// A3：`Retry-After: 60` 的权威窗口必须胜过正文里的 `retry_after=999999`。
///
/// 组合的是**冻结的**两个公开函数：`append_retry_after_marker`（唯一装配点）+
/// `rate_limit_window_ms`（唯一解析点）。缺陷链：
/// 1. 正文含 `retry_after=` ⇒ `append_retry_after_marker` 的幂等判定短路，**60s 根本写不进文本**；
/// 2. `parse_retry_after_marker` 取首个出现 ⇒ 解析到正文的 999999s（≈11.5 天）。
/// 结果：一次 429 能把 provider 冷却近 11.5 天（远超 6h 上界前的取值链）。
#[test]
fn adversarial_retry_after_marker_must_not_be_hijacked_by_response_body() {
    // Arrange：状态为 429、响应头 Retry-After: 60；正文被上游塞入 retry_after=999999。
    const BODY_WITH_POISON: &str = "{\"error\":{\"type\":\"exceeded_current_quota_error\",\"message\":\"You've reached your 5-hour usage limit\",\"note\":\"retry_after=999999\"}}";
    let base = format!(
        "provider 返回错误状态：429 Too Many Requests；耗时=4031ms；响应正文：{}",
        BODY_WITH_POISON
    );
    // 生产装配点（唯一）：权威头值 60s 必须进入文本。
    let error = append_retry_after_marker(&base, 60);

    // Act
    let window_ms = rate_limit_window_ms(&error, (0, 0));
    let hint = rate_limit_recovery_hint(&error);
    println!("[PA-171 A3] error = {error}");
    println!("[PA-171 A3] window_ms = {window_ms:?} / hint = {hint:?}");

    // Assert：必须取权威头值 60s —— 不得取正文的 999999s。
    assert_eq!(
        window_ms,
        Some(60_000),
        "A3：retry_after 被正文劫持——应按响应头 60s 冷却，实际 {window_ms:?}\
         （正文 poison = retry_after=999999 ⇒ 999_999_000ms ≈ 11.5 天，会把 provider 冻死）"
    );
    assert_ne!(
        window_ms,
        Some(999_999_000),
        "A3：绝不允许把上游正文里的 retry_after=999999 当成权威窗口（正文不可信）"
    );
    // 上界：窗口不得超过矩阵 C3-P5 的 6h 封顶（60s 远小于上界，锁住"另算上界"也满足）。
    assert!(
        window_ms.map(|value| value <= 6 * 60 * 60 * 1_000).unwrap_or(false),
        "A3：冷却窗口必须被 6h 上界约束，实际 {window_ms:?}"
    );
    // 唯一解析点旁证：人读恢复时间与窗口同源，必须同为 60s。
    assert_eq!(
        hint.as_deref(),
        Some("预计 60s 后恢复"),
        "A3：人读恢复时间必须与窗口同源（60s），实际 {hint:?}"
    );
    // 装配点旁证：权威头值必须真的写进了文本（幂等短路导致的丢失必须在断言里显形）。
    // 只断言"权威值在场"，不锁它排在正文 poison 之前还是之后（那是实现自由）。
    assert!(
        error.contains(&format!("{RETRY_AFTER_MARKER}60")),
        "A3：装配点必须把权威 Retry-After=60 写入错误文本（不得被正文同名 token 短路），实际 {error:?}"
    );
}
