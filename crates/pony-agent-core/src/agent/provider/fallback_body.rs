//! B221/B254：provider 本地兜底的**用户文案层**（与内部诊断严格分层）。
//!
//! 唯一落点：单行人读正文（谁做了什么 + 没拿到什么 + 怎么继续）。内部诊断
//! （URL / `elapsed=` / provider source / 堆栈）只进 trace 与日志的 `fallback_reason`。
//!
//! B254 简化：按产品要求「面向用户的文案必须通俗简明，无必要不显示」，正文不再拼接
//! 摘要片段与链路解释，也从 `pub` 面移除摘要消毒/截断工具（连带删除其单测——那些断言
//! 校验的正是已删除行为）。

use super::{
    estimate_token_usage, preview_text, provider_native_assistant_message, ProviderRequest,
    ProviderResponse, ToolCall, ToolResult,
};

/// provider 本地兜底正文的**唯一构造点**（单行人读文案，`pub` 供集成验收调用）。
///
/// 硬约束：
/// - **单行一句话**：点名工具 + 「已执行完成」事实 + 一句可执行建议；
/// - **不含内部细节**：`error` 只用于 [`fallback_error_category`] 归类，原文绝不进正文；
/// - **工具名只做不可见字符剥离 + 单行折叠**：MCP 工具名本来就含 `.`/`/`
///   （`mcp__server__tool.v2`、`fs/read`），不能按 URL/路径过滤，否则正文退化成「该工具」；
/// - `summary` 形参保留兼容旧调用面，**不参与正文**（B254：摘要字眼对用户无意义）。
pub fn fallback_body_text(tool_name: &str, error: &str, summary: &str) -> String {
    let _ = summary;
    let scrubbed_tool = single_line_identifier(tool_name);
    let tool: &str = if scrubbed_tool.is_empty() {
        "该工具"
    } else {
        scrubbed_tool.as_str()
    };
    format!(
        "`{tool}` 已执行完成，但模型没有回复（{}）。发送「继续」接着做。",
        fallback_error_category(error)
    )
}

/// `error` 字符串 → 原因**类别**（只输出短分类词，绝不回显 `error` 原文）。
///
/// 判定来源收归 [`crate::agent::retry::classify_error_class`] 的稳定语义码；
/// 例外只有一条：我们自己产的 body 解析失败标记是结构化前缀，单独归类为「回复异常」。
fn fallback_error_category(error: &str) -> &'static str {
    let lowered = error.to_ascii_lowercase();
    if lowered.contains("decoding response body") || lowered.contains("解析 provider 返回失败") {
        return "回复异常";
    }
    match crate::agent::retry::classify_error_class(error) {
        "timeout" => "超时",
        "network" => "网络中断",
        "rate_limited" => "请求过多",
        "quota" => "额度用尽",
        "auth" => "登录失效",
        "bad_request" => "请求无效",
        _ => "未完成",
    }
}

/// 会破坏字面匹配的零宽/格式字符（U+200B–200F / U+202A–202E / U+2060–2064 / U+FEFF）。
///
/// 剥离而非替换成空格：零宽字符本就不该占位；工具名里夹带它们会让「点名」读不出来。
fn is_invisible_format_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}'
    )
}

/// 把**结构化标识**（工具名）折叠成单行——剥离零宽/格式字符 + 空白折叠。
fn single_line_identifier(fragment: &str) -> String {
    fragment
        .chars()
        .filter(|ch| !is_invisible_format_char(*ch))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// provider 本地兜底响应（`provider_followup_local_fallback`）的**唯一构造点**。
///
/// 正文只放人读文案（[`fallback_body_text`]）；`error` 原文只进 `fallback_reason`
/// （trace / provider 日志）——正文会被落库成历史消息，内部细节不得污染后续上下文。
///
/// 注：B254 后流式 followup 不再走此兜底；本构造点保留给非流式 provider 与旧回放。
pub fn local_tool_followup_fallback_response(
    request: &ProviderRequest,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
    error: String,
) -> ProviderResponse {
    let fallback_reason = format!("provider_followup_failed: {}", preview_text(&error, 240));
    let summary = super::local_tool_followup_summary(tool_result);
    let output_text = fallback_body_text(tool_call.name.as_str(), &error, &summary);

    ProviderResponse {
        output_text: output_text.clone(),
        tool_calls: Vec::new(),
        tool_call: None,
        reasoning_content: None,
        reasoning_content_value: None,
        assistant_message: Some(provider_native_assistant_message(&output_text)),
        provider_source: "provider_followup_local_fallback".to_string(),
        provider_mode: "fallback".to_string(),
        fallback_reason: Some(fallback_reason),
        token_usage: Some(estimate_token_usage(request, &output_text)),
    }
}

#[cfg(test)]
mod tests {
    use super::{fallback_body_text, fallback_error_category};

    /// 公共不变量：单行、非空、不含 URL / 计时 / 域名 / 技术词。
    fn assert_user_facing(text: &str) {
        assert!(!text.trim().is_empty(), "正文不得为空：{text:?}");
        assert!(!text.contains('\n'), "正文必须是单行：{text:?}");
        for forbidden in [
            "https://",
            "http://",
            "elapsed=",
            "tokens.ponyjob.top",
            "上游",
            "兜底",
            "provider",
        ] {
            assert!(!text.contains(forbidden), "正文不得含 {forbidden:?}：{text:?}");
        }
    }

    #[test]
    fn categories_are_mapped_from_error_text() {
        // B254：用户可见类别用短词（超时 / 网络中断 / 回复异常 / 请求过多 / 未完成）。
        assert_eq!(fallback_error_category("type=timeout+request"), "超时");
        assert_eq!(fallback_error_category("request TIMED OUT"), "超时");
        assert_eq!(fallback_error_category("connect error: dns lookup failed"), "网络中断");
        assert_eq!(
            fallback_error_category(
                "调用 provider 失败：error sending request；type=request；elapsed=26951ms"
            ),
            "网络中断"
        );
        assert_eq!(fallback_error_category("connection reset by peer"), "网络中断");
        assert_eq!(
            fallback_error_category("error decoding response body: unexpected eof"),
            "回复异常"
        );
        assert_eq!(fallback_error_category("HTTP 429 rate limit exceeded"), "请求过多");
        assert_eq!(fallback_error_category("boom"), "未完成");
    }

    #[test]
    fn body_is_one_plain_sentence() {
        let text = fallback_body_text(
            "Write",
            "error sending request for url (https://tokens.ponyjob.top/v1/chat/completions)；type=timeout+request；elapsed=21161ms",
            "已执行 Write 工具 https://example.com/secret elapsed=99ms",
        );
        assert_user_facing(&text);
        assert_eq!(text, "`Write` 已执行完成，但模型没有回复（超时）。发送「继续」接着做。");
    }

    #[test]
    fn empty_summary_and_unknown_error_stay_readable() {
        let text = fallback_body_text("Bash", "boom", "   ");
        assert_user_facing(&text);
        assert!(text.contains("Bash"), "未知原因也必须点名工具：{text:?}");
        assert!(text.contains("未完成"), "未知原因归类为「未完成」：{text:?}");
        assert!(text.contains("发送「继续」"), "必须给可执行建议：{text:?}");
    }

    /// 含 `.` / `/` 的 MCP 工具名必须原样点名（不做 URL/路径过滤）。
    #[test]
    fn dotted_mcp_tool_name_is_kept_verbatim() {
        let text = fallback_body_text("mcp__server__tool.v2", "type=timeout+request", "");
        assert_user_facing(&text);
        assert!(
            text.contains("mcp__server__tool.v2"),
            "MCP 工具名必须逐字保留：{text:?}"
        );
    }

    /// 空工具名退化为「该工具」，但仍是一句完整的人话。
    #[test]
    fn blank_tool_name_degrades_to_generic_noun() {
        let text = fallback_body_text("   ", "boom", "");
        assert_user_facing(&text);
        assert!(text.contains("该工具"), "空工具名必须有兜底称谓：{text:?}");
        assert!(text.contains("发送「继续」"), "必须给可执行建议：{text:?}");
    }
}
