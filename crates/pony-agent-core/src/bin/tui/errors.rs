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

/// PA-171 C5：限流/配额类终态摘要的**固定开头**（动态文案以前缀判定严重级）。
pub const ERROR_SUMMARY_RATE_LIMITED: &str = "上游限流/配额受限";

/// PA-171 C5：无可用恢复窗口时的完整文案。
pub const ERROR_SUMMARY_RATE_LIMITED_NO_WINDOW: &str =
    "上游限流/配额受限，恢复时间未知，请稍后重试。";

/// PA-171 C5-S2：限流/配额命中时的可读摘要（含恢复时间）。
///
/// 恢复时间与冷却窗口同源（`retry::rate_limit_recovery_hint`，唯一解析点）；
/// 解析不到时如实说明"恢复时间未知"，**不得**折叠为 `ERROR_SUMMARY_TRANSIENT`。
pub fn rate_limited_error_summary(error: &str) -> String {
    match pony_agent_core::agent::retry::rate_limit_recovery_hint(error) {
        Some(hint) => format!("{ERROR_SUMMARY_RATE_LIMITED}，{hint}。"),
        None => ERROR_SUMMARY_RATE_LIMITED_NO_WINDOW.to_string(),
    }
}

/// PA-172 §5（遗留 #6）：注入 `now` 的版本——恢复提示按**真实 now** 折算剩余窗口
/// （`retry::rate_limit_recovery_hint_at`，唯一解析点；与冷却窗口同源）。
pub fn rate_limited_error_summary_at(error: &str, now_unix_ms: u64) -> String {
    match pony_agent_core::agent::retry::rate_limit_recovery_hint_at(error, now_unix_ms) {
        Some(hint) => format!("{ERROR_SUMMARY_RATE_LIMITED}，{hint}。"),
        None => ERROR_SUMMARY_RATE_LIMITED_NO_WINDOW.to_string(),
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

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
        ERROR_SUMMARY_GENERIC
            | ERROR_SUMMARY_TRANSIENT
            | ERROR_SUMMARY_AUTH
            | ERROR_SUMMARY_RATE_LIMITED
            | ERROR_SUMMARY_RATE_LIMITED_NO_WINDOW
    ) {
        return SystemMessageSeverity::Error;
    }
    // PA-171 C5-S4：限流/配额摘要是动态文案（含恢复时间）⇒ 按固定开头判定。
    if trimmed.starts_with(ERROR_SUMMARY_RATE_LIMITED) {
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
pub fn user_facing_error_summary(error: &str) -> String {
    user_facing_error_summary_at(error, now_unix_ms())
}

/// PA-171 C5-S1：注入 `now` 的同一摘要入口（时钟可注入，避免测试等待真实窗口）。
///
/// 判定顺序冻结：`auth` → 限流/配额（`retry::classify_error_class` ∈
/// {`rate_limited`,`quota`}）→ `is_transient_provider_error` → 通用文案。
/// 限流分支必须早于 transient：否则 429 会被折叠成「重试已用尽」（历史缺陷 F13）。
pub fn user_facing_error_summary_at(error: &str, now_unix_ms: u64) -> String {
    // L3-B BLOCKING：auth 只认**头部段**（`；响应正文` 之前）的词边界判据，禁止裸
    // `contains` 扫全文——`429 … 耗时=4031ms` / `… 耗时=401ms` / 正文含 "api key"
    // 都会被裸 contains 误判成凭据错误（真因是限流/配额）。
    if pony_agent_core::agent::retry::is_auth_error(error) {
        return ERROR_SUMMARY_AUTH.to_string();
    }
    if matches!(
        pony_agent_core::agent::retry::classify_error_class(error),
        "rate_limited" | "quota"
    ) {
        // PA-172 §5（遗留 #6）：`now` 是**真参数**——恢复提示按真实时刻折算剩余窗口
        // （历史缺陷：形参被 `let _ = now_unix_ms;` 丢弃，文案不随时刻收敛）。
        return rate_limited_error_summary_at(error, now_unix_ms);
    }
    if is_transient_provider_error(error) {
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
        ERROR_SUMMARY_GENERIC
            | ERROR_SUMMARY_TRANSIENT
            | ERROR_SUMMARY_AUTH
            | ERROR_SUMMARY_RATE_LIMITED
            | ERROR_SUMMARY_RATE_LIMITED_NO_WINDOW
    ) {
        return false;
    }
    // PA-171 C5：限流/配额摘要是动态文案（固定开头 + 恢复时间）⇒ 从不折叠。
    if trimmed.starts_with(ERROR_SUMMARY_RATE_LIMITED) {
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
///
/// **不得在此复制实现**：真值源是 `pony_agent_core::agent::session::interrupted_turn_checkpoint_text`
/// （`session/store.rs`）。此处曾有一份逐字复制品，「改一处漏一处」的风险由 L2 收口
/// （同一事实一个家）。保留本入口只为 C1–C8 契约测试的调用面稳定。
pub fn interrupted_turn_checkpoint(
    prior_assistant_text: &str,
    tool_activities: &[String],
) -> Option<String> {
    pony_agent_core::agent::session::interrupted_turn_checkpoint_text(
        prior_assistant_text,
        tool_activities,
    )
}

/// 取消终态的**一行状态**（面向用户；过程清单不进正文）。
pub const INTERRUPTED_CANCELLED_LINE: &str = "上一轮已停止，发送消息可继续。";

/// 上游中断/崩溃收口的**一行状态**（面向用户；步骤明细由 trace 工具行承载）。
pub const INTERRUPTED_FAILED_LINE: &str = "上一轮中断，已保留已执行的步骤记录。";

const CHECKPOINT_MARKER: &str = "[中断轮次检查点]";

/// L2（dsh 对齐）：合成中断记录的一行状态文案。
///
/// 判定顺序：结构化 `kind` **优先**；旧会话无 kind ⇒ 回退 `content` 标记嗅探。
/// 两条路径给出同一句文案——这正是「载体种类」成为结构化事实后，展示层不再猜文本的理由。
///
/// `None` ⇒ 这是模型真实产出，调用方按正文渲染。
pub fn interrupted_record_status_line(
    content: &str,
    kind: Option<pony_agent_core::agent::session::HistoryMessageKind>,
) -> Option<&'static str> {
    use pony_agent_core::agent::session::HistoryMessageKind;
    match kind {
        Some(HistoryMessageKind::Cancelled) => Some(INTERRUPTED_CANCELLED_LINE),
        Some(HistoryMessageKind::Interrupted) => Some(INTERRUPTED_FAILED_LINE),
        None => {
            let trimmed = content.trim();
            if trimmed == pony_agent_core::agent::session::CANCELLED_TURN_SENTINEL {
                Some(INTERRUPTED_CANCELLED_LINE)
            } else if trimmed.starts_with(CHECKPOINT_MARKER) {
                Some(INTERRUPTED_FAILED_LINE)
            } else {
                None
            }
        }
    }
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

#[cfg(test)]
mod interrupted_record_status_tests {
    use super::*;
    use pony_agent_core::agent::session::HistoryMessageKind;

    const CHECKPOINT: &str = "[中断轮次检查点]\n已执行的工具步骤：\n- Run 探测";

    /// 结构化 `kind` 优先：同一段落库文本，两种终态给出不同的一句话。
    #[test]
    fn kind_takes_precedence_over_text() {
        assert_eq!(
            interrupted_record_status_line(CHECKPOINT, Some(HistoryMessageKind::Cancelled)),
            Some(INTERRUPTED_CANCELLED_LINE)
        );
        assert_eq!(
            interrupted_record_status_line(CHECKPOINT, Some(HistoryMessageKind::Interrupted)),
            Some(INTERRUPTED_FAILED_LINE)
        );
    }

    /// 旧会话无 `kind`：按落库标记回退，文案与 kind 路径一致（零迁移）。
    #[test]
    fn legacy_rows_fall_back_to_marker_sniffing() {
        assert_eq!(
            interrupted_record_status_line(CHECKPOINT, None),
            Some(INTERRUPTED_FAILED_LINE)
        );
        assert_eq!(
            interrupted_record_status_line(
                pony_agent_core::agent::session::CANCELLED_TURN_SENTINEL,
                None
            ),
            Some(INTERRUPTED_CANCELLED_LINE)
        );
    }

    /// 普通正文不得被改写（前缀判定，不是包含判定）。
    #[test]
    fn plain_answer_and_marker_suffix_are_untouched() {
        assert_eq!(interrupted_record_status_line("正常的模型回答", None), None);
        assert_eq!(
            interrupted_record_status_line(&format!("前置说明\n{CHECKPOINT}"), None),
            None
        );
    }
}
