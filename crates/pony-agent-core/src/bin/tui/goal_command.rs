//! 契约 C-6：`/goal` 斜杠命令的 TUI 接线。
//!
//! 分界线（与桌面端同口径）：**语法、状态机与回执文案的唯一真源在 Rust core**
//! （`pony_agent_core::agent::goal_command`）；本模块只做两件事——
//! ① 从输入框原文提取参数；② 把执行结果投递到 TUI（System 消息 + 状态行 + goal 横幅）。
//! 命令不消耗模型轮次、不写会话历史语义，也不发起 turn。

use super::*;

/// `/goal` 之后的参数文本；未命中 `/goal` ⇒ `None`。
///
/// 口径（C-6.2）：首 token 大小写不敏感等于 `/goal`（复用 [`match_command_token`]，
/// 与命令表同源，禁止另写一份字面量表）；返回其余文本（可为空串，代表 `show`）。
/// `/goa`、`/goalx`、`/goalx 目标` 一律 `None`——它们仍按普通消息发送。
pub fn goal_command_args(input: &str) -> Option<String> {
    if match_command_token(input) != Some("/goal") {
        return None;
    }

    let trimmed = input.trim();
    Some(
        trimmed
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim().to_string())
            .unwrap_or_default(),
    )
}

/// 执行 `/goal` 并把回执投递到 TUI；返回是否命中该命令（调用方据此 `continue`）。
///
/// - `Ok` 且带回目标快照 ⇒ 横幅按真实快照更新；
/// - `Ok` 且无快照（`clear` / 无目标的 `show`）⇒ 横幅立即消失，保证「清除后不再有 armed goal」
///   的可见性与 `active_goal_for_session` 同源；
/// - `Err` ⇒ System 消息 + 状态行如实呈现（不得 panic、不得静默、不得谎报成功）。
pub fn handle_goal_command(state: &mut TuiState, input: &str) -> bool {
    let Some(args) = goal_command_args(input) else {
        return false;
    };

    match pony_agent_core::agent::goal_command::execute_goal_command(&state.session_id, &args) {
        Ok(outcome) => {
            match outcome.goal {
                Some(goal) => state.set_goal(tui_goal_from_goal(&goal)),
                None => state.clear_goal_banner(),
            }
            state.set_status_notice(single_line_notice(&outcome.message));
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: outcome.message,
                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            });
        }
        Err(error) => {
            let receipt = format!("目标命令执行失败：{error}");
            state.set_status_notice(receipt.clone());
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: receipt,
                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            });
        }
    }

    true
}

/// 状态行是**单行**通道（C-6.3）：多行回执折叠为 ` · `，既不丢内容也不撑破布局。
fn single_line_notice(message: &str) -> String {
    message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_command_args_matches_only_exact_token() {
        assert_eq!(goal_command_args("/goal"), Some(String::new()));
        assert_eq!(goal_command_args("  /goal  "), Some(String::new()));
        assert_eq!(goal_command_args("/GOAL"), Some(String::new()));
        assert_eq!(goal_command_args("/goal pause"), Some("pause".to_string()));
        assert_eq!(
            goal_command_args("/goal edit 更小的目标  "),
            Some("edit 更小的目标".to_string())
        );
        // 正文不得被吞：首 token 命中即可带走全部剩余文本。
        assert_eq!(
            goal_command_args("/goal 完成迁移并跑通门禁"),
            Some("完成迁移并跑通门禁".to_string())
        );
    }

    #[test]
    fn goal_command_args_rejects_near_misses() {
        assert_eq!(goal_command_args("/goa"), None);
        assert_eq!(goal_command_args("/goalx"), None);
        assert_eq!(goal_command_args("/goalx 目标"), None);
        assert_eq!(goal_command_args("goal 目标"), None);
        assert_eq!(goal_command_args("/plan"), None);
        assert_eq!(goal_command_args("/clear 这个需求怎么拆"), None);
        assert_eq!(goal_command_args(""), None);
    }

    #[test]
    fn single_line_notice_folds_multiline_receipt() {
        let folded = single_line_notice("目标状态: 进行中\n目标: 迁移\n\n轮次: 0/30");
        assert_eq!(folded, "目标状态: 进行中 · 目标: 迁移 · 轮次: 0/30");
        assert!(!folded.contains('\n'));
    }
}
