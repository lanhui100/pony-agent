//! B221：provider 本地兜底的**用户文案层**（与内部诊断严格分层）。
//!
//! 唯一落点：单行人读正文的构造（工具名 + 已执行事实 + 原因**类别** + 建议动作 + 短摘要）。
//! 内部诊断（URL / `elapsed=` / provider source / 堆栈）只进 trace 与日志的 `fallback_reason`，
//! **不得**出现在本文件产出的正文里——正文会被落库成历史消息，污染后续上下文与剪枝判定。
//!
//! 独立成文件的理由：`provider/mod.rs` 已触规模门禁上限，且「用户文案」与「HTTP/协议层」
//! 本就不该混居（改文案不该动传输代码）。

/// B221：本地兜底摘要进正文的字符上限（含省略号）——超长摘要不得把单行正文撑开。
const FALLBACK_SUMMARY_MAX_CHARS: usize = 120;

/// B221：provider 本地兜底正文的**唯一构造点**（单行人读文案，`pub` 供集成验收调用）。
///
/// 组成（顺序固定）：点名工具 + 「已执行完成」事实 + 原因**类别** + 一句可执行建议 +
/// 可选短摘要。硬约束：
/// - **单行**：所有空白折叠为单空格，绝不产出换行；
/// - **不含内部细节**：`error` 只用于 [`fallback_error_category`] 归类，原文**绝不进正文**；
///   URL / `elapsed=` / provider source / 堆栈一类技术串一律剥离（它们是 trace 的载体）；
/// - 摘要按 [`FALLBACK_SUMMARY_MAX_CHARS`] 按 `char` 截断（不切 UTF-8 码点）。
pub fn fallback_body_text(tool_name: &str, error: &str, summary: &str) -> String {
    let scrubbed_tool = scrub_fallback_fragment(tool_name);
    let tool: &str = if scrubbed_tool.is_empty() {
        "该工具"
    } else {
        scrubbed_tool.as_str()
    };
    let mut text = format!(
        "工具 `{tool}` 已执行完成，但 provider 在整合工具结果时失败（原因：{}），本轮未产出最终回复。",
        fallback_error_category(error)
    );
    let digest = scrub_fallback_fragment(summary);
    if !digest.is_empty() {
        text.push_str(&format!(
            "本地兜底摘要：{}。",
            truncate_chars(&digest, FALLBACK_SUMMARY_MAX_CHARS)
        ));
    }
    text.push_str("建议下一步：先确认该工具的实际结果，再决定是否重试或继续下一步。");
    text
}

/// B221：`error` 字符串 → 原因**类别**（只输出分类词，绝不回显 `error` 原文）。
///
/// 判据顺序即优先级：先超时（`timeout` 常见于连接串里），再连接（connect/dns/network），
/// 再响应解析（decode/body），其余归「上游返回异常」。
fn fallback_error_category(error: &str) -> &'static str {
    let lowered = error.to_ascii_lowercase();
    let has = |needle: &str| lowered.contains(needle);
    if has("timeout") || has("timed out") {
        "上游响应超时"
    } else if has("connect") || has("dns") || has("network") {
        "连接上游失败"
    } else if has("decode") || has("body") {
        "上游响应解析失败"
    } else {
        "上游返回异常"
    }
}

/// B221：正文片段消毒——剥离 URL / 计时 / provider source 技术串，并折叠为单行。
///
/// 实现取向是「删 token 而不是删子串」：命中禁词的那**一个空白分隔片段**整段丢弃，
/// 避免残留半截 URL（`https://host/x` 被切出 `https:` 之类）或半截计时串。
fn scrub_fallback_fragment(fragment: &str) -> String {
    fragment
        .split_whitespace()
        .filter(|token| {
            let lowered = token.to_ascii_lowercase();
            !lowered.contains("://")
                && !lowered.contains("elapsed=")
                && !lowered.contains("provider_source")
                && !lowered.starts_with("www.")
                && !lowered.starts_with("http")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// B221：按 `char` 截断（不切 UTF-8 码点），超限时含 `…` 总长 ≤ `max_chars`。
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut truncated: String = text.chars().take(max_chars.saturating_sub(1)).collect();
    truncated.push('…');
    truncated
}

#[cfg(test)]
mod tests {
    use super::{fallback_body_text, fallback_error_category, FALLBACK_SUMMARY_MAX_CHARS};

    /// 公共不变量：单行 + 不含 URL / 计时 / provider 域名。
    fn assert_user_facing(text: &str) {
        assert!(!text.trim().is_empty(), "兜底正文不得为空：{text:?}");
        assert!(!text.contains('\n'), "兜底正文必须是单行：{text:?}");
        for forbidden in ["https://", "http://", "elapsed=", "tokens.ponyjob.top"] {
            assert!(
                !text.contains(forbidden),
                "兜底正文不得泄漏内部细节 {forbidden:?}：{text:?}"
            );
        }
    }

    #[test]
    fn categories_are_mapped_from_error_text() {
        // 超时 / 连接 / 解析三类必须各自命中（含大小写混写）。
        assert_eq!(fallback_error_category("type=timeout+request"), "上游响应超时");
        assert_eq!(fallback_error_category("request TIMED OUT"), "上游响应超时");
        assert_eq!(fallback_error_category("connect error: dns lookup failed"), "连接上游失败");
        assert_eq!(fallback_error_category("network unreachable"), "连接上游失败");
        assert_eq!(
            fallback_error_category("error decoding response body: unexpected eof"),
            "上游响应解析失败"
        );
        assert_eq!(fallback_error_category("boom"), "上游返回异常");
    }

    #[test]
    fn body_is_single_line_and_strips_sensitive_details() {
        let text = fallback_body_text(
            "Write",
            "error sending request for url (https://tokens.ponyjob.top/v1/chat/completions)；type=timeout+request；elapsed=21161ms",
            "已执行 Write 工具 https://example.com/secret elapsed=99ms",
        );
        assert_user_facing(&text);
        assert!(text.contains("Write"), "必须点名工具：{text:?}");
        assert!(text.contains("超时"), "timeout 必须归类为超时：{text:?}");
        assert!(text.contains("已执行完成"), "必须含已执行语义：{text:?}");
        assert!(text.contains("建议下一步"), "必须含可执行建议：{text:?}");
    }

    #[test]
    fn long_summary_is_compressed_into_one_line() {
        let summary = "写".repeat(2_000);
        let text = fallback_body_text("Write", "type=timeout+request", &summary);
        assert_user_facing(&text);
        assert!(
            text.chars().count() < 400,
            "2000 字符摘要必须压成短预览：{} 字符",
            text.chars().count()
        );
        assert!(
            text.contains(&"写".repeat(FALLBACK_SUMMARY_MAX_CHARS - 1)),
            "摘要应保留前 {FALLBACK_SUMMARY_MAX_CHARS} 字符量级：{text:?}"
        );
    }

    #[test]
    fn empty_summary_and_unknown_error_stay_readable() {
        let text = fallback_body_text("Bash", "boom", "   ");
        assert_user_facing(&text);
        assert!(text.contains("Bash"), "未知原因也必须点名工具：{text:?}");
        assert!(text.contains("异常"), "未知原因归上游返回异常：{text:?}");
        assert!(
            !text.contains("本地兜底摘要"),
            "空摘要不得留空段：{text:?}"
        );
    }
}
