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
///
/// C4/C5（wave-interrupted-turn-resume）：展示侧走 [`display_dsml_protocol_text`] —— 除既有
/// 折叠外，还丢弃行首就是协议语法的碎片行（`parameter name=…` 之类），并把连排占位符收敛为
/// 一个。**持久化正文不得经本入口**（占位符会写回落库），落库侧走
/// [`crate::agent::session::interrupted_turn::persisted_assistant_text`]。
#[allow(unused_variables)]
pub fn sanitize_model_protocol_text(text: &str) -> (String, bool) {
    // 半截标记先截掉（流式 chunk 边界），再交给 provider 层同一纯函数做块级折叠；
    // 普通正文与 markdown 代码块逐字保真。
    let (body, incomplete) = match dsml_partial_tail_len(text) {
        Some(len) => (&text[..text.len() - len], true),
        None => (text, false),
    };
    let cleaned = crate::agent::provider::display_dsml_protocol_text(body);
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
    match crate::agent::retry::rate_limit_recovery_hint(error) {
        Some(hint) => format!("{ERROR_SUMMARY_RATE_LIMITED}，{hint}。"),
        None => ERROR_SUMMARY_RATE_LIMITED_NO_WINDOW.to_string(),
    }
}

/// PA-172 §5（遗留 #6）：注入 `now` 的版本——恢复提示按**真实 now** 折算剩余窗口
/// （`retry::rate_limit_recovery_hint_at`，唯一解析点；与冷却窗口同源）。
pub fn rate_limited_error_summary_at(error: &str, now_unix_ms: u64) -> String {
    match crate::agent::retry::rate_limit_recovery_hint_at(error, now_unix_ms) {
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
///
/// **结构化来源优先**：凡文案由领域枚举生成（如 goal 续轮停止原因），必须走
/// [`severity_for_goal_stop`] 而**不要**把文案回灌本函数——关键词表对「首轮未成功」
/// 这类不含 error 词的失败句必然判错（历史缺陷：为可见性新增的消息成了权重最低的一条）。
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
    // 结构化来源优先：凡文案由领域枚举生成（goal 停止族），直接取枚举严重级，
    // **不得**落到下面的关键词表——「首轮失败，自动续轮未启动」不含 error 词，必被判错。
    if let Some(severity) = goal_stop_severity_for_line(trimmed) {
        return severity;
    }
    // PA-171 C5-S4：限流/配额摘要是动态文案（含恢复时间）⇒ 按固定开头判定。
    if trimmed.starts_with(ERROR_SUMMARY_RATE_LIMITED) {
        return SystemMessageSeverity::Error;
    }
    // B189 复审：Ask 收口文案按**生命周期**分派（中性句，不再含「失效」）。它们是「需要注意但
    // 不是错误」的受控文案，必须显式判 Warning —— 靠关键词表会因文案改造静默降级成 Info。
    if matches!(
        trimmed,
        ASK_STALE_NOTICE
            | ASK_SETTLED_ANSWERED_NOTICE
            | ASK_SETTLED_CANCELLED_NOTICE
            | ASK_SETTLED_SUPERSEDED_NOTICE
            | ASK_SETTLED_ORPHANED_NOTICE
            | ASK_STILL_PENDING_NOTICE
            | ASK_LIFECYCLE_READ_FAILED_NOTICE
    ) {
        return SystemMessageSeverity::Warning;
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

/// 按**领域枚举**识别一条 goal 停止文案的严重级（`None` = 不是 goal 停止文案）。
///
/// 为什么不复用关键词表：`FirstRoundNotSucceeded` 的文案里没有 `error`/`失败` 之外的
/// error 词，关键词表必然把它降级成 Info 灰字。这里不做语义猜测，只拿
/// [`GoalRoundStopReason::all_variants`] 的**逐字文案**做身份比对——文案与严重级同出
/// 一个枚举，改文案时两边一起动，不存在漂移。
pub fn goal_stop_severity_for_line(
    content: &str,
) -> Option<SystemMessageSeverity> {
    use crate::agent::goal::GoalRoundStopReason;
    let trimmed = content.trim();
    GoalRoundStopReason::all_variants()
        .into_iter()
        .find(|reason| reason.visible_line() == trimmed)
        .map(|reason| severity_for_goal_stop(reason.severity()))
}

/// 领域枚举 → 渲染严重级的**唯一映射**（枚举驱动，禁止关键词猜测）。
///
/// 为什么必须独立于 [`system_message_severity`]：后者靠文案关键词判级，而
/// `FirstRoundNotSucceeded` 的文案「首轮失败，自动续轮未启动」不含 `error`/`失败`
/// 之外的任何 error 词，实测被降级成 Info 灰字——**为可见性新增的消息反而最不醒目**。
/// 领域枚举本身就是真值，无需再猜；这里只做一次纯翻译。
pub fn severity_for_goal_stop(
    severity: crate::agent::goal::GoalStopSeverity,
) -> SystemMessageSeverity {
    use crate::agent::goal::GoalStopSeverity;
    match severity {
        GoalStopSeverity::Info => SystemMessageSeverity::Info,
        GoalStopSeverity::Warning => SystemMessageSeverity::Warning,
        GoalStopSeverity::Error => SystemMessageSeverity::Error,
    }
}

/// 严重级 → 文本样式（消息区与状态栏共用，避免两处配色漂移）。
///
/// `fallback` 供调用方保留原有「无严重级」配色（如消息区 Info 用 `muted()`、
/// 状态栏 Info 用 `muted()`/`faint()`）。
///
/// 2026-11-14（用户裁决）：`Error` **不再叠加 `BOLD`** —— DANGER 前景色本身已是醒目
/// 警示色，再加字重属于「加重显示」，且与消息区/状态行同时出同一句时更像重复强调。
/// 区分度靠颜色（Error=DANGER / Warning=WARN / Info=fallback），不靠字重。
pub fn severity_style(severity: SystemMessageSeverity, fallback: Style) -> Style {
    match severity {
        SystemMessageSeverity::Error => Style::default().fg(theme::DANGER),
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
    if crate::agent::retry::is_auth_error(error) {
        return ERROR_SUMMARY_AUTH.to_string();
    }
    if matches!(
        crate::agent::retry::classify_error_class(error),
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
/// **不得在此复制实现**：真值源是 `crate::agent::session::interrupted_turn_checkpoint_text`
/// （`session/store.rs`）。此处曾有一份逐字复制品，「改一处漏一处」的风险由 L2 收口
/// （同一事实一个家）。保留本入口只为 C1–C8 契约测试的调用面稳定。
pub fn interrupted_turn_checkpoint(
    prior_assistant_text: &str,
    tool_activities: &[String],
) -> Option<String> {
    crate::agent::session::interrupted_turn_checkpoint_text(
        prior_assistant_text,
        tool_activities,
    )
}

/// 取消终态的**一行状态**（面向用户；过程清单不进正文）。
pub const INTERRUPTED_CANCELLED_LINE: &str = "上一轮已停止，发送消息可继续。";

/// 上游中断/崩溃收口的**一行状态**（面向用户；步骤明细由 trace 工具行承载）。
///
/// C4：措辞必须显式表达"**已中断**"（与取消轮的"已停止"可区分，且是单行一句话）。
pub const INTERRUPTED_FAILED_LINE: &str = "上一轮已中断，已保留已执行的步骤记录。";

/// B217：本轮**真的失败**（上游报错 / 工具链失败）收口的一行状态。
///
/// 与 [`INTERRUPTED_FAILED_LINE`] 严格区分：把 Error 说成「已中断」会误导续跑判断。
pub const INTERRUPTED_ERROR_LINE: &str = "上一轮因错误终止，已保留已执行的步骤记录。";

/// B274：降级收尾（**跑完了、降级交付**）的一行状态。
///
/// 措辞刻意不说「中断」「失败」：降级轮有交付物，只是质量打折（provider 兜底摘要）。
/// 用户据此判断的是「内容可能不完整」，而不是「需要重跑」。
pub const DEGRADED_TURN_LINE: &str = "上一轮以降级方式收尾，已保留已执行的步骤记录。";

const CHECKPOINT_MARKER: &str = "[中断轮次检查点]";

/// resume 回放时该不该在信息流末尾补一行中断状态（`None` = 不补）。
///
/// 判据是「**历史末条**是否为中断载体」，不是「历史里最后一条能匹配的记录」。
/// 正常完成的 assistant 行没有信号（`kind = None` 且正文不以检查点标记开头 ⇒
/// [`interrupted_record_status_line`] 返回 `None`）；若反向逐条 `find_map`，扫描会越过
/// 它继续往前翻，把若干轮之前的旧失败重新贴到末尾 —— 于是「上一轮明明成功」也报
/// 「因错误终止」。末条不是 assistant（末轮尚未产出回复，或宿主补录了指令）时同样不报：
/// 此时不存在「上一轮」这个可断言的对象。
///
/// 独立成函数是为了让「历史形状 → 是否补行」成为可机械断言的唯一判定点。
pub fn resume_interrupted_status_line(
    history: &[crate::agent::session::TurnHistoryMessage],
) -> Option<&'static str> {
    history
        .last()
        .filter(|message| message.role == "assistant")
        .and_then(|message| interrupted_record_status_line(&message.content, message.kind))
}

/// L2（dsh 对齐）：合成中断记录的一行状态文案。
///
/// 判定顺序：结构化 `kind` **优先**；旧会话无 kind ⇒ 回退 `content` 标记嗅探。
/// 两条路径给出同一句文案——这正是「载体种类」成为结构化事实后，展示层不再猜文本的理由。
///
/// `None` ⇒ 这是模型真实产出，调用方按正文渲染。
pub fn interrupted_record_status_line(
    content: &str,
    kind: Option<crate::agent::session::HistoryMessageKind>,
) -> Option<&'static str> {
    use crate::agent::session::HistoryMessageKind;
    match kind {
        Some(HistoryMessageKind::Cancelled) => Some(INTERRUPTED_CANCELLED_LINE),
        Some(HistoryMessageKind::Interrupted) => Some(INTERRUPTED_FAILED_LINE),
        // B217：失败不是中断 —— 文案必须给出不同的续跑判断依据。
        Some(HistoryMessageKind::Error) => Some(INTERRUPTED_ERROR_LINE),
        // B274：降级是「已交付但打折」，第三种续跑判断。
        Some(HistoryMessageKind::Degraded) => Some(DEGRADED_TURN_LINE),
        None => {
            let trimmed = content.trim();
            if trimmed == crate::agent::session::CANCELLED_TURN_SENTINEL {
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
    use crate::agent::session::HistoryMessageKind;

    const CHECKPOINT: &str = "[中断轮次检查点]\n已执行的工具步骤：\n- Run 探测";

    /// B217：失败不得被读成「已中断」——两种终态给出不同的续跑判断文案。
    #[test]
    fn error_kind_is_not_reported_as_interrupted() {
        assert_eq!(
            interrupted_record_status_line(CHECKPOINT, Some(HistoryMessageKind::Error)),
            Some(INTERRUPTED_ERROR_LINE)
        );
        assert_ne!(
            INTERRUPTED_ERROR_LINE, INTERRUPTED_FAILED_LINE,
            "两种终态必须给出可区分的一句话"
        );
    }

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
                crate::agent::session::CANCELLED_TURN_SENTINEL,
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

    /// B274：降级轮是第四种终态——不得冒充「已中断」，也不得冒充「因错误终止」。
    ///
    /// 三种非正常收口各给一句不同的话：中断 = 没跑完；错误 = 失败无交付；
    /// 降级 = 跑完了但质量打折。共用一个 kind 会让用户按错误方式续跑。
    #[test]
    fn degraded_kind_has_its_own_line() {
        assert_eq!(
            interrupted_record_status_line(CHECKPOINT, Some(HistoryMessageKind::Degraded)),
            Some(DEGRADED_TURN_LINE)
        );
        assert_ne!(DEGRADED_TURN_LINE, INTERRUPTED_FAILED_LINE);
        assert_ne!(DEGRADED_TURN_LINE, INTERRUPTED_ERROR_LINE);
        // 措辞纪律：降级轮有交付物，不得说成「中断」或「失败」。
        assert!(
            !DEGRADED_TURN_LINE.contains("中断") && !DEGRADED_TURN_LINE.contains("失败"),
            "降级文案不得自称中断/失败：{DEGRADED_TURN_LINE}"
        );
    }

    /// B274：`Degraded` 的 wire 值双向可达；未知值 fail-soft 落到最保守的 `Interrupted`。
    #[test]
    fn degraded_kind_wire_round_trips_and_unknown_is_fail_soft() {
        let wire = serde_json::to_string(&HistoryMessageKind::Degraded).unwrap();
        assert_eq!(wire, "\"degraded\"");
        let parsed: HistoryMessageKind = serde_json::from_str("\"degraded\"").unwrap();
        assert_eq!(parsed, HistoryMessageKind::Degraded);
        // 旧二进制读新值不炸开：未知 wire 落到 Interrupted（非正常收口的保守桶）。
        let unknown: HistoryMessageKind = serde_json::from_str("\"some_future_kind\"").unwrap();
        assert_eq!(unknown, HistoryMessageKind::Interrupted);
    }
}
