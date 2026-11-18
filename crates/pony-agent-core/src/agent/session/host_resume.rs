//! B235 / B238：宿主续轮（Ask 恢复 / 审批恢复 / goal 轮次推进）在**会话历史**里的落库角色
//! 与「组合正文」识别的单一真值源。
//!
//! - B235：宿主续轮不是用户发言。历史行以 [`HOST_HISTORY_ROLE`] 落库，不再占 `user` 槽；
//!   模型面由 `agent::context::to_provider_history_message` 投影成 `developer` 消息，
//!   续轮指令对模型仍然可见。
//! - B238：B232 把「【挂起前用户指令（宿主补录）】+ 宿主文案」组合成正文。新写入按 host 角色
//!   落库；**旧日志**里这条组合行仍是 `user` 角色，[`is_composed_host_continuation`] 让既有
//!   派生（标题 / `turn_count` / `continuation_only` / `needs_ai_title`）把它当续轮跳过。
//!
//! schema 决策：**不新增 `HistoryMessageKind` 变体**——该枚举只服务 assistant 合成记录，
//! `append_turn` 用 `kind.is_some()` 决定 TurnCancelled 节点语义，复用会改坏轮次收口；
//! 改用 additive 的 role 白名单扩展（`role` 本就是自由字符串，旧日志无该值 ⇒ 逐字回放）。

use super::store::is_host_continuation_prompt;

/// 宿主上下文历史行角色（additive role 白名单：`user` / `assistant` 之外的新值）。
/// 旧读者读到未知角色会按既有 `_ =>` 分支忽略或跳过，不改变旧日志回放。
pub const HOST_HISTORY_ROLE: &str = "host";

/// 组合正文段标：被挂起回合的原用户指令（契约 C6/C7，逐字冻结）。
pub const RESUME_INSTRUCTION_MARKER: &str = "【挂起前用户指令（宿主补录）】";
/// 组合正文段标：宿主续轮文案。
pub const RESUME_PROMPT_MARKER: &str = "【宿主续轮】";

/// 是否为「原指令 + 宿主续轮文案」包围块：原指令段标开头且宿主续轮段标在场。
///
/// 判据刻意宽于逐字匹配：原指令是用户真实文本（任意长度/换行），只有两个段标是结构事实。
pub fn is_composed_host_continuation(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with(RESUME_INSTRUCTION_MARKER) && trimmed.contains(RESUME_PROMPT_MARKER)
}

/// 历史行角色决策（写路径唯一口径）：宿主续轮 ⇒ [`HOST_HISTORY_ROLE`]，其余 ⇒ `user`。
pub fn user_history_role(user_content: &str) -> &'static str {
    if is_host_continuation_prompt(user_content) {
        HOST_HISTORY_ROLE
    } else {
        "user"
    }
}

/// 组合正文的唯一构造口（TUI 生产与派生识别共用同一版式，防两处文案漂移）。
pub fn compose_host_continuation(instruction: &str, prompt: &str) -> String {
    format!("{RESUME_INSTRUCTION_MARKER}\n{instruction}\n\n{RESUME_PROMPT_MARKER}\n{prompt}")
}
