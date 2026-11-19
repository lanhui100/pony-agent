//! B221 本地兜底正文验收（红相）：用户文案只保留「工具已执行 + 原因类别 + 建议动作」一行，
//! 内部诊断细节（URL / `elapsed=` / provider 域名）只在 trace 与日志里出现。
//!
//! 现场：`provider/mod.rs` 的兜底模板把 `preview_text(error, 240)` 原样拼进 assistant 正文，
//! 用户看到的是 `error sending request for url (https://tokens.ponyjob.top/...)；
//! type=timeout+request；elapsed=21161ms；…`，并落库成为历史消息（污染后续上下文与剪枝判定）。
//!
//! B254 修订（产品要求「面向用户简化」）：正文压成**一句话**——点名工具 + 已执行事实 +
//! 短类别词 + 「发送「继续」接着做」；不再拼接摘要片段。本文件按新契约重锚类别词
//! （超时 / 网络 / 回复），其余不变量（单行、非空、无 URL/elapsed/域名、点名工具）不变。
//!
//! 本文件全部为真实断言：不读真实时钟、不 sleep、不吞异常。

use pony_agent_core::agent::provider::fallback_body_text;

/// 内部诊断串在正文里**一律**不得出现的子串（URL / 计时 / provider 域名）。
const FORBIDDEN_DETAILS: [&str; 4] = ["https://", "http://", "elapsed=", "tokens.ponyjob.top"];

/// 公共不变量：单行、非空、不含内部细节。
fn assert_user_facing_single_line(label: &str, text: &str) {
    assert!(
        !text.trim().is_empty(),
        "{label}：兜底正文不得为空串/纯空白，实际 = {text:?}"
    );
    assert!(
        !text.contains('\n'),
        "{label}：兜底正文必须是单行（多行会破坏 TUI 逐字回放与状态行高度），实际 = {text:?}"
    );
    for forbidden in FORBIDDEN_DETAILS {
        assert!(
            !text.contains(forbidden),
            "{label}：兜底正文不得泄漏内部细节 {forbidden:?}，实际 = {text:?}"
        );
    }
}

/// 1. 真实现场串（URL + type + elapsed）必须被压成人读单行，且保留工具名与「超时」类别。
#[test]
fn b221_timeout_body_is_single_line_without_internal_details() {
    // Arrange：与其指纹一致的现场 error 串（含 URL 与 elapsed）。
    const TOOL: &str = "Write";
    let raw_error = "error sending request for url (https://tokens.ponyjob.top/v1/chat/completions)；type=timeout+request；elapsed=21161ms";
    let summary = "已执行 Write 工具，等待模型总结时上游无响应";

    // Act
    let text = fallback_body_text(TOOL, raw_error, summary);

    // Assert 1：单行、非空、无内部细节（含现场 URL 与 elapsed）。
    assert_user_facing_single_line("timeout", &text);

    // Assert 2：必须点名工具与原因类别。
    assert!(
        text.contains(TOOL),
        "兜底正文必须点名已执行的工具 {TOOL}，实际 = {text:?}"
    );
    assert!(
        text.contains("超时"),
        "timeout 类错误必须归类为「超时」，实际 = {text:?}"
    );
}

/// 2. connect/dns/network ⇒ 连接类（不得含糊成「上游返回异常」）。
#[test]
fn b221_connect_error_classification() {
    // Arrange
    const TOOL: &str = "Bash";
    let raw_error = "connect error: dns lookup failed";

    // Act
    let text = fallback_body_text(TOOL, raw_error, "命令已执行");

    // Assert
    assert_user_facing_single_line("connect", &text);
    assert!(
        text.contains("网络"),
        "connect/dns 类错误必须归类为「网络中断」，实际 = {text:?}"
    );
    assert!(
        text.contains(TOOL),
        "兜底正文必须点名已执行的工具 {TOOL}，实际 = {text:?}"
    );
}

/// 3. decode/body ⇒ 解析类。
#[test]
fn b221_decode_error_classification() {
    // Arrange
    const TOOL: &str = "Run";
    let raw_error = "error decoding response body: unexpected eof";

    // Act
    let text = fallback_body_text(TOOL, raw_error, "探测已执行");

    // Assert
    assert_user_facing_single_line("decode", &text);
    assert!(
        text.contains("回复"),
        "decode/body 类错误必须归类为「回复异常」，实际 = {text:?}"
    );
    assert!(
        text.contains(TOOL),
        "兜底正文必须点名已执行的工具 {TOOL}，实际 = {text:?}"
    );
}

/// 4. 无法归类的原因不得产出空串（退化必须保守且仍可读）。
#[test]
fn b221_unknown_error_falls_back_to_generic_category() {
    // Arrange
    const TOOL: &str = "Bash";

    // Act
    let text = fallback_body_text(TOOL, "boom", "");

    // Assert
    assert_user_facing_single_line("unknown", &text);
    assert!(
        text.contains(TOOL),
        "未知原因也必须点名工具（空泛文案等于没有信息），实际 = {text:?}"
    );
    assert!(
        text.chars().count() > TOOL.chars().count(),
        "正文不得只剩工具名，实际 = {text:?}"
    );
}

/// 5. 超长 summary 不得把正文撑开：仍是一行且总长受限（≤400 字符）。
#[test]
fn b221_summary_preview_stays_short() {
    // Arrange
    const TOOL: &str = "Write";
    let long_summary = "写".repeat(2_000);
    assert_eq!(
        long_summary.chars().count(),
        2_000,
        "前提：夹具 summary 为 2000 字符"
    );

    // Act
    let text = fallback_body_text(TOOL, "type=timeout+request", &long_summary);

    // Assert
    assert_user_facing_single_line("long-summary", &text);
    let length = text.chars().count();
    assert!(
        length <= 400,
        "2000 字符 summary 必须被压成短预览（≤400 字符），实际 = {length} 字符：{text:?}"
    );
    assert!(
        text.contains(TOOL),
        "压缩后仍必须点名已执行的工具 {TOOL}，实际 = {text:?}"
    );
}
