//! 上游错误分类与协议文本清洗（原 `tui.rs` 内联实现，机械外移）。
//! 契约：DSML 协议原文清洗（C1/C2）、瞬态错误判定（C3）、用户可见错误摘要与折叠判据。

use super::*;

pub const DSML_CALLS_OPEN: &str = "<｜｜DSML｜｜ calls>";

pub const DSML_CALLS_CLOSE: &str = "</｜｜DSML｜｜ calls>";

/// 只把「已识别 DSML 标记前缀」的半截形态判为 incomplete；普通 `<` / 代码块不误伤。
pub fn dsml_partial_tail_len(text: &str) -> Option<usize> {
    let trimmed = text.trim_end_matches(['\r', '\n', ' ', '\t']);
    let tail_start = trimmed.rfind('<')?;
    let tail = &trimmed[tail_start..];
    if tail.contains('>') {
        return None;
    }
    // 单个 `<` 是普通正文，不是协议前缀；至少要有 `<` 后的一个字符才算候选。
    if tail.len() <= "<".len() {
        return None;
    }
    let known_tags = [
        DSML_CALLS_OPEN,
        "<｜｜DSML｜｜ invoke",
        "<｜｜DSML｜｜ parameter",
        DSML_CALLS_CLOSE,
        "</｜｜DSML｜｜ invoke>",
        "</｜｜DSML｜｜ parameter>",
    ];
    known_tags
        .iter()
        .any(|tag| tag.starts_with(tail))
        .then_some(text.len() - tail_start)
}

/// C1/C2：清洗上游模型输出内的 DSML 工具调用协议原文（流式与历史回放共用）。
/// 返回：清洗后的用户可见文本；`incomplete=true` 表示尾部可能是被 chunk 切断的协议片段。
#[allow(unused_variables)]
pub fn sanitize_model_protocol_text(text: &str) -> (String, bool) {
    // 半截标记先截掉（流式 chunk 边界），再交给 provider 层同一纯函数做块级折叠；
    // 普通正文与 markdown 代码块逐字保真。
    let (body, incomplete) = match dsml_partial_tail_len(text) {
        Some(len) => (&text[..text.len() - len], true),
        None => (text, false),
    };
    let cleaned = pony_agent_core::agent::provider::sanitize_dsml_protocol_text(body);
    (cleaned, incomplete)
}

/// C3：判定上游错误是否为可重试瞬态错误（与 provider 层既有预算对齐，不新增重试）。
#[allow(unused_variables)]
pub fn is_transient_provider_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    let permanent = [
        "401", "403", "400", "unauthorized", "forbidden", "bad request",
        "missing api key", "api key", "invalid_api_key", "nonretryable",
    ];
    if permanent.iter().any(|hint| lower.contains(hint)) {
        return false;
    }
    let transient = [
        "timeout", "timed out", "429", "rate limit", "rate-limited", "503",
        "502", "504", "500", "5xx", "connection refused", "connection reset",
        "dns", "network", "connection", "temporarily unavailable",
    ];
    transient.iter().any(|hint| lower.contains(hint))
}

pub const ERROR_SUMMARY_GENERIC: &str = "本轮请求未能完成，请稍后再试。";

/// 瞬态上游错误的**终态**文案。
///
/// 历史缺陷：此处曾写「正在按既有预算重试」，而本常量唯一的消费点是
/// `UiEvent::StreamFailed` 臂 —— 该事件由 `emit_stream_failed` 在 provider 重试预算
/// **终止之后**发出。文案与语义脱节：用户看到「正在重试」，但该轮已无任何重试在跑。
pub const ERROR_SUMMARY_TRANSIENT: &str = "上游服务暂时不可用，重试已用尽。";

pub const ERROR_SUMMARY_AUTH: &str = "凭据不可用，请检查提供商配置。";

/// 系统消息的严重级：渲染样式（图标 / 颜色）的唯一判据。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemMessageSeverity {
    Info,
    Warning,
    Error,
}

/// 纯函数：由系统消息正文判定严重级。
///
/// 为什么不是 `ChatMessage` 的结构字段：`ChatMessage` 被多个验收测试直接构造
/// （`tests/tui_*_acceptance.rs`），加字段会连带改动冻结用例。故把判据收敛为本函数，
/// 消息区渲染与状态栏共用同一真值源，替代此前散落在三处的 `contains("失败")` 关键词
/// 判定 —— `ERROR_SUMMARY_*` 文案不含任何既有关键词，导致真实的终态错误反而被渲染成
/// 灰色 Info 样式（这就是「错误信息不醒目」的根因）。
pub fn system_message_severity(content: &str) -> SystemMessageSeverity {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return SystemMessageSeverity::Info;
    }
    // 面向用户的错误摘要是**受控文案**：逐字命中即 Error，不依赖正文用词。
    if matches!(
        trimmed,
        ERROR_SUMMARY_GENERIC | ERROR_SUMMARY_TRANSIENT | ERROR_SUMMARY_AUTH
    ) {
        return SystemMessageSeverity::Error;
    }
    let lower = trimmed.to_ascii_lowercase();
    let error_hints = [
        "error", "fail", "panic", "失败", "错误", "异常", "不可用", "未产出终态",
    ];
    if error_hints.iter().any(|hint| lower.contains(hint)) {
        return SystemMessageSeverity::Error;
    }
    let warning_hints = ["warn", "警告", "已过期", "已失效"];
    if warning_hints.iter().any(|hint| lower.contains(hint)) {
        return SystemMessageSeverity::Warning;
    }
    SystemMessageSeverity::Info
}

/// 严重级 → 文本样式（消息区与状态栏共用，避免两处配色漂移）。
///
/// `fallback` 供调用方保留原有「无严重级」配色（如消息区 Info 用 `muted()`、
/// 状态栏 Info 用 `muted()`/`faint()`）。
pub fn severity_style(severity: SystemMessageSeverity, fallback: Style) -> Style {
    match severity {
        SystemMessageSeverity::Error => {
            Style::default().fg(theme::DANGER).add_modifier(Modifier::BOLD)
        }
        SystemMessageSeverity::Warning => Style::default().fg(theme::WARN),
        SystemMessageSeverity::Info => fallback,
    }
}

/// C4：把原始错误详情收敛为面向用户的一句话摘要（详情默认折叠，不入消息区）。
#[allow(unused_variables)]
pub fn user_facing_error_summary(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("401")
        || lower.contains("403")
        || lower.contains("api key")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
    {
        ERROR_SUMMARY_AUTH.to_string()
    } else if is_transient_provider_error(error) {
        ERROR_SUMMARY_TRANSIENT.to_string()
    } else {
        ERROR_SUMMARY_GENERIC.to_string()
    }
}

/// C4：判定错误详情是否应默认折叠（终态错误 true；面向用户摘要 false）。
#[allow(unused_variables)]
pub fn should_fold_error_detail(error: &str) -> bool {
    let trimmed = error.trim();
    if matches!(
        trimmed,
        ERROR_SUMMARY_GENERIC | ERROR_SUMMARY_TRANSIENT | ERROR_SUMMARY_AUTH
    ) {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    let internal = [
        "error sending request for url", "http://", "https://", "{\"error\"",
        "stacktrace", "response body", "响应正文", "panic", "at provider::",
    ];
    trimmed.chars().count() > 200 || internal.iter().any(|hint| lower.contains(hint))
}

/// C5/C6：把中断 turn 的已落盘步骤投影为可续传检查点（向后兼容：空输入返回 None）。
#[allow(unused_variables)]
pub fn interrupted_turn_checkpoint(
    prior_assistant_text: &str,
    tool_activities: &[String],
) -> Option<String> {
    let text = prior_assistant_text.trim();
    let tools: Vec<&str> = tool_activities
        .iter()
        .map(|tool| tool.trim())
        .filter(|tool| !tool.is_empty())
        .collect();
    if text.is_empty() && tools.is_empty() {
        return None;
    }
    let mut checkpoint = String::from("[中断轮次检查点]");
    if !text.is_empty() {
        checkpoint.push('\n');
        checkpoint.push_str(text);
    }
    if !tools.is_empty() {
        checkpoint.push_str("\n已执行的工具步骤：");
        for tool in tools {
            checkpoint.push_str("\n- ");
            checkpoint.push_str(tool);
        }
    }
    Some(checkpoint)
}

/// C6：resume 时从检查点接续，返回应追加到已完成历史之后的可见文本；不重复已完成 turn。
#[allow(unused_variables)]
pub fn resume_from_checkpoint(completed_history: &str, checkpoint: Option<&str>) -> String {
    let Some(checkpoint) = checkpoint else {
        return completed_history.to_string();
    };
    let checkpoint = checkpoint.trim();
    if checkpoint.is_empty() || completed_history.contains(checkpoint) {
        return completed_history.to_string();
    }
    let mut resumed = String::with_capacity(completed_history.len() + checkpoint.len() + 1);
    resumed.push_str(completed_history);
    if !resumed.is_empty() && !resumed.ends_with('\n') {
        resumed.push('\n');
    }
    resumed.push_str(checkpoint);
    resumed
}
