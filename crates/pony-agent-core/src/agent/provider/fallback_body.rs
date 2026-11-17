//! B221：provider 本地兜底的**用户文案层**（与内部诊断严格分层）。
//!
//! 唯一落点：单行人读正文的构造（工具名 + 已执行事实 + 原因**类别** + 建议动作 + 短摘要）。
//! 内部诊断（URL / `elapsed=` / provider source / 堆栈）只进 trace 与日志的 `fallback_reason`，
//! **不得**出现在本文件产出的正文里——正文会被落库成历史消息，污染后续上下文与剪枝判定。
//!
//! 独立成文件的理由：`provider/mod.rs` 已触规模门禁上限，且「用户文案」与「HTTP/协议层」
//! 本就不该混居（改文案不该动传输代码）。

use super::{
    estimate_token_usage, local_tool_followup_summary, preview_text,
    provider_native_assistant_message, ProviderRequest, ProviderResponse, ToolCall, ToolResult,
};

/// B221：本地兜底摘要进正文的字符上限（含省略号）——超长摘要不得把单行正文撑开。
const FALLBACK_SUMMARY_MAX_CHARS: usize = 120;

/// B221：provider 本地兜底正文的**唯一构造点**（单行人读文案，`pub` 供集成验收调用）。
///
/// 组成（顺序固定）：点名工具 + 「已执行完成」事实 + 原因**类别** + 一句可执行建议 +
/// 可选短摘要。硬约束：
/// - **单行**：所有空白折叠为单空格，绝不产出换行；
/// - **不含内部细节**：`error` 只用于 [`fallback_error_category`] 归类，原文**绝不进正文**；
///   URL / `elapsed=` / provider source / 堆栈一类技术串一律剥离（它们是 trace 的载体）；
/// - **工具名只做不可见字符剥离 + 单行折叠**（见 [`single_line_identifier`]）：MCP 工具名本来就
///   含 `.`/`/`（`mcp__server__tool.v2`、`fs/read`），走 summary 的 URL/路径 token 过滤会被整段
///   丢弃，正文退化成「该工具」——那等于没点名，违背本函数的第一条组成要求；
/// - 摘要按 [`FALLBACK_SUMMARY_MAX_CHARS`] 按 `char` 截断（不切 UTF-8 码点）。
pub fn fallback_body_text(tool_name: &str, error: &str, summary: &str) -> String {
    let scrubbed_tool = single_line_identifier(tool_name);
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

/// A3：会破坏字面匹配的零宽/格式字符（U+200B–200F / U+202A–202E / U+2060–2064 / U+FEFF）。
///
/// 攻击形态：`ht\u{200B}tps://host/x`、`h\u{FEFF}ttps`——不先剥离就会绕过 `://` / `http` 判据，
/// 把整条 URL 当成「普通 token」带进正文并落库。剥离而非替换成空格：零宽字符本来就不该占位，
/// 合并两侧字母后才能让 `https` / `://` 判据命中。
fn is_invisible_format_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2064}' | '\u{FEFF}'
    )
}

/// A3：单个 token 是否承载内部细节（fail-closed，命中任一判据即整段丢弃）。
///
/// 判据面：URL scheme（`://` / `http…` / `www.`）、路径与凭据（`/` / `@`）、计时键值
/// （`elapsed=`）、域名 / IP / 文件名（含 `.` 且非纯十进制数）、provider 内部标记。
/// 纯数字与 `status=error` 这类**无人读敌意**的普通键值保留——正文要有信息量，不是越空越安全。
fn token_is_internal_detail(token: &str) -> bool {
    let lowered = token.to_ascii_lowercase();
    lowered.contains("://")
        || lowered.starts_with("http")
        || lowered.starts_with("www.")
        || lowered.contains("provider_source")
        || lowered.contains("elapsed=")
        || token.contains('/')
        || token.contains('@')
        || (token.contains('.') && !is_plain_decimal(token))
}

/// A3：纯十进制数（`3` / `1.5` / `-2`）——数字本身不是内部细节，只有域名/IP/文件名才丢弃。
///
/// 至多**一个**小数点：`1.5` 是数值，`127.0.0.1` 是 IP（内部细节），二者不能同判。
fn is_plain_decimal(token: &str) -> bool {
    let body = token.strip_prefix('-').unwrap_or(token);
    !body.is_empty()
        && body.chars().filter(|ch| *ch == '.').count() <= 1
        && body.chars().all(|ch| ch.is_ascii_digit() || ch == '.')
}

/// A3/L3：把**结构化标识**（工具名）折叠成单行——剥离零宽/格式字符 + 空白折叠，
/// **不做** token 级 URL/路径过滤。
///
/// 与 [`scrub_fallback_fragment`] 的分工是刻意的：工具名是「点名」用的标识，`. `/`/`_` 是
/// 其正常语法（`mcp__server__tool.v2`、`fs/read`）；摘要才可能夹带 URL / 域名 / 计时串。
/// 两处共用同一套 token 判据会互相误伤（要么丢工具名，要么放行摘要里的 host）。
fn single_line_identifier(fragment: &str) -> String {
    fragment
        .chars()
        .filter(|ch| !is_invisible_format_char(*ch))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// B221/A3：正文片段消毒——先剥离零宽格式字符，再丢弃承载内部细节的 token，折叠为单行。
///
/// 实现取向是「删 token 而不是删子串」：命中禁词的**一个空白分隔片段**整段丢弃，
/// 避免残留半截 URL（`https://host/x` 被切出 `https:` 之类）或半截计时串。
/// 空白被 `split_whitespace` 吃掉后，`https :// host` 这种「拆开写」的形态会分别命中
/// `http…` 前缀与 `://` 判据——三个片段一起丢弃，不留残余。
fn scrub_fallback_fragment(fragment: &str) -> String {
    let stripped: String = fragment
        .chars()
        .filter(|ch| !is_invisible_format_char(*ch))
        .collect();
    stripped
        .split_whitespace()
        .filter(|token| !token_is_internal_detail(token))
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

/// B221：provider 本地兜底响应（`provider_followup_local_fallback`）的**唯一构造点**。
///
/// 正文只放人读文案（[`fallback_body_text`]）；`error` 原文只进 `fallback_reason`
/// （trace / provider 日志）——正文会被落库成历史消息，内部细节（URL / `elapsed=` /
/// 域名）不得污染后续上下文。
///
/// 迁自 `provider/mod.rs`：该文件已触文件规模基线（`npm run size:check`），且本函数与
/// [`fallback_body_text`] 同属「本地兜底」语义面（改文案不该动传输代码）。
pub fn local_tool_followup_fallback_response(
    request: &ProviderRequest,
    tool_call: &ToolCall,
    tool_result: &ToolResult,
    error: String,
) -> ProviderResponse {
    let fallback_reason = format!("provider_followup_failed: {}", preview_text(&error, 240));
    let summary = local_tool_followup_summary(tool_result);
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

    /// A3-1：零宽字符插入 URL（`ht\u{200B}tps://…`）必须被识别并整段剥离。
    #[test]
    fn zero_width_obfuscated_url_is_stripped() {
        let summary = "已执行 https://exa\u{200B}mple.com/secret\u{FEFF}path 命令";
        let text = fallback_body_text("Bash", "boom", summary);
        assert_user_facing(&text);
        for forbidden in ["exa", "mple.com", "secret", "://", "https"] {
            assert!(
                !text.contains(forbidden),
                "零宽混淆的 URL 片段 {forbidden:?} 不得进正文：{text:?}"
            );
        }
        assert!(text.contains("命令"), "无害摘要仍需保留：{text:?}");
    }

    /// A3-2：scheme 与 host 被空白拆开（`https :// host`）时三个片段都要丢弃。
    #[test]
    fn whitespace_split_scheme_and_host_are_stripped() {
        let summary = "上游 https :// tokens.ponyjob.top/v1 无响应";
        let text = fallback_body_text("Bash", "type=timeout+request", summary);
        assert_user_facing(&text);
        for forbidden in ["https", "://", "tokens.ponyjob.top", "ponyjob"] {
            assert!(
                !text.contains(forbidden),
                "拆分书写的 {forbidden:?} 不得进正文：{text:?}"
            );
        }
        assert!(text.contains("超时"), "原因类别仍需保留：{text:?}");
    }

    /// A3-3：裸 host（无 scheme、无路径）同样不得进正文。
    #[test]
    fn bare_host_token_is_stripped() {
        let summary = "主机 tokens.ponyjob.top 拒绝连接";
        let text = fallback_body_text("Bash", "connect error", summary);
        assert_user_facing(&text);
        assert!(
            !text.contains("tokens.ponyjob.top") && !text.contains("ponyjob"),
            "裸 host 不得进正文：{text:?}"
        );
        assert!(text.contains("主机"), "无害摘要仍需保留：{text:?}");
        assert!(text.chars().count() <= 400, "压成单行后总长受限：{text:?}");
    }

    /// A3-4：含 `.` 的 MCP 工具名必须**原样点名**（路径/域名判据只作用于摘要）。
    #[test]
    fn dotted_mcp_tool_name_is_kept_verbatim() {
        let text = fallback_body_text("mcp__server__tool.v2", "type=timeout+request", "");
        assert_user_facing(&text);
        assert!(
            text.contains("mcp__server__tool.v2"),
            "点号工具名必须逐字出现：{text:?}"
        );
        assert!(!text.contains("`该工具`"), "不得回落占位工具名：{text:?}");
    }

    /// A3-5：含 `/` 的工具名同样原样保留（`fs/read` 不得被当成路径片段丢弃）。
    #[test]
    fn slashed_tool_name_is_kept_verbatim() {
        let text = fallback_body_text("fs/read", "boom", "");
        assert_user_facing(&text);
        assert!(text.contains("fs/read"), "斜杠工具名必须逐字出现：{text:?}");
        assert!(!text.contains("`该工具`"), "不得回落占位工具名：{text:?}");
    }
}
