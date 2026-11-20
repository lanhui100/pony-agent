//! B193（2026-11-15）：**首轮门**非成功终态 → 可见停止原因的真值表。
//!
//! PA-171 C1-T1 规定「只有首轮真正成功收口才进入自动续轮」，但旧实现在首轮
//! `TurnStreamOutcome` 非 `Completed` 时**裸 `return`** —— 既不产生
//! `UiEvent::GoalRoundsStopped`，也不写状态行与消息区。用户看到的只是「这一轮结束了」，
//! 误判会话终止，需要手动发「继续」才发现目标还能推进（症状取证见 backlog B193）。
//!
//! 本模块把「首轮终态 → 停止原因」收成**纯函数**（唯一判定点，可机械断言），
//! 调用方只负责把结果交给 `report_goal_rounds_stopped`（可见性由既有通道承担）。

use pony_agent_core::agent::control_plane::TurnStreamOutcome;
use pony_agent_core::agent::goal::GoalRoundStopReason;

/// 首轮终态 → 「不再进入自动续轮」的可见原因（`None` = 首轮成功，继续走续轮循环）。
///
/// 映射口径（与 [`GoalRoundStopReason`] 既有变体逐一对齐，不新增语义重叠的变体）：
/// - `Completed` ⇒ `None`（进入 `run_goal_rounds`）
/// - `Cancelled` ⇒ `Cancelled`（用户停止，最高优先级原因）
/// - `Suspended` ⇒ `TurnSuspended`（模型挂起等 Ask/审批，续轮必须让位）
/// - `Unknown` ⇒ `TurnUnclassified`（不知道成没成，不当成功也不冒充失败）
/// - `Failed` ⇒ `FirstRoundNotSucceeded`（有失败终态、但续轮从未启动；
///   不得复用 `ConsecutiveFailures`——那读作「已经连跑了好几轮」）
pub fn first_round_stop_reason(outcome: &TurnStreamOutcome) -> Option<GoalRoundStopReason> {
    match outcome {
        TurnStreamOutcome::Completed => None,
        TurnStreamOutcome::Cancelled => Some(GoalRoundStopReason::Cancelled),
        TurnStreamOutcome::Suspended => Some(GoalRoundStopReason::TurnSuspended),
        TurnStreamOutcome::Unknown => Some(GoalRoundStopReason::TurnUnclassified),
        TurnStreamOutcome::Failed { .. } => Some(GoalRoundStopReason::FirstRoundNotSucceeded),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed() -> TurnStreamOutcome {
        TurnStreamOutcome::Failed {
            error: Some("provider SendRequest".to_string()),
        }
    }

    #[test]
    fn completed_outcome_enters_goal_rounds() {
        assert_eq!(first_round_stop_reason(&TurnStreamOutcome::Completed), None);
    }

    #[test]
    fn cancelled_outcome_reports_cancelled() {
        assert_eq!(
            first_round_stop_reason(&TurnStreamOutcome::Cancelled),
            Some(GoalRoundStopReason::Cancelled)
        );
    }

    #[test]
    fn suspended_outcome_reports_turn_suspended() {
        assert_eq!(
            first_round_stop_reason(&TurnStreamOutcome::Suspended),
            Some(GoalRoundStopReason::TurnSuspended)
        );
    }

    #[test]
    fn unknown_outcome_reports_turn_unclassified() {
        assert_eq!(
            first_round_stop_reason(&TurnStreamOutcome::Unknown),
            Some(GoalRoundStopReason::TurnUnclassified)
        );
    }

    #[test]
    fn failed_outcome_reports_first_round_not_succeeded() {
        assert_eq!(
            first_round_stop_reason(&failed()),
            Some(GoalRoundStopReason::FirstRoundNotSucceeded)
        );
    }

    /// 文案是用户唯一能读到的结论：首轮门用「未启动」，不得冒充「已停止」。
    ///
    /// b193-2 起文案去术语化（不再出现「收口」），但**语义锁不变**：必须说「未启动」，
    /// 且绝不能出现「已停止」——这是 B193 决策记录冻结的区分，不随措辞改造回退。
    #[test]
    fn first_round_not_succeeded_line_says_not_started() {
        let line = GoalRoundStopReason::FirstRoundNotSucceeded.visible_line();
        assert!(line.contains("未启动"), "实际文案：{line}");
        assert!(
            !line.contains("已停止"),
            "首轮门从未进入续轮，不得说「已停止」：{line}"
        );
        // 严重级必须由枚举给：旧关键词表把它判成 Info 灰字（B193 目的被反向实现）。
        assert_eq!(
            GoalRoundStopReason::FirstRoundNotSucceeded.severity(),
            pony_agent_core::agent::goal::GoalStopSeverity::Error,
            "首轮失败必须是 Error 级：{line}"
        );
        // 必须给出可执行动作，否则用户仍只能干等或乱试。
        assert!(
            line.contains("继续"),
            "停止文案必须含下一步动作（发「继续」）：{line}"
        );
    }
}
