//! B129：goal 终态（complete / blocked）的**收尾上下文**。
//!
//! 问题：模型在自己发起的 goal 轮里把目标标成 `completed` / `blocked` 后，宿主循环因为
//! `active_goal_for_session` 不再命中而**硬停**；模型本轮没有收到任何「把结论写给用户」的
//! 指令，于是经常出现「标记完成但用户读不到结果」。
//!
//! 对照 DSH：`packages/goal/tool-goal/src/wrapup.ts:17-40` 产出
//! `<goal_complete>` / `<goal_blocked>` 引导，`index.ts:318-330` 用 `deferContext` 注入。
//! Pony 没有 `deferContext` 这一层，但工具结果的回灌通道就是同一件事：把收尾指令追加进
//! `update_goal` 的 tool result（`tools/orchestration.rs::execute_update_goal`），模型下一次
//! 采样必然看到，且不新增第二套注入机制。
//!
//! 「终态后不再发起新轮」不是本模块新增的闸门，而是既有续轮判据的结构性结果：
//! 两个宿主循环（`control_plane::execute_graph_run_stream` / `advance_graph_run`）每轮都重新
//! 读 [`continuation_goal_for_run`](super::goal::continuation_goal_for_run)，TUI 循环每轮都重新
//! 读 [`active_goal_for_session`](super::goal::active_goal_for_session)；两者都只命中
//! `phase == "active" && activation == Armed`。因此本模块只负责**文案**，并用
//! [`is_terminal_phase`] 保证终态识别与续轮读面同一口径（单测里直接断言真实读面返回 `None`）。

use super::goal::GoalData;

/// 终态相位集合（与 `goal::active_goal_for_session` 的过滤互补）。
pub fn is_terminal_phase(phase: &str) -> bool {
    matches!(phase, "completed" | "blocked")
}

/// 目标处于终态时返回收尾指令；其余相位返回 `None`（调用方直接跳过注入）。
pub fn wrapup_for_goal(goal: &GoalData) -> Option<String> {
    match goal.phase.as_str() {
        "completed" => Some(complete_wrapup(&goal.objective)),
        "blocked" => Some(blocked_wrapup(
            &goal.objective,
            goal.blocked_reason.as_deref(),
        )),
        _ => None,
    }
}

/// complete 分支：已完成什么、还剩什么、本轮不再调工具 / 不再开新轮。
fn complete_wrapup(objective: &str) -> String {
    format!(
        "<goal_complete>目标已标记为完成。请用这一段最终答复收尾：总结目标「{}」实际完成了什么（关键改动、产物与验证结果），并说明仍未覆盖或需要用户确认的点。本轮不要再调用工具，也不要再发起新的目标轮次（终态后不再续轮）。</goal_complete>",
        objective.trim()
    )
}

/// blocked 分支：阻塞原因、已完成部分、解除阻塞的最小动作。
fn blocked_wrapup(objective: &str, blocked_reason: Option<&str>) -> String {
    let reason = blocked_reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .unwrap_or("未提供具体原因");
    format!(
        "<goal_blocked>目标已标记为阻塞（原因：{reason}）。请用这一段最终答复收尾：说明阻塞原因、目标「{}」已经完成的部分，以及解除阻塞所需的最小用户动作或下一步建议。本轮不要再调用工具，也不要再发起新的目标轮次（终态后不再续轮）。</goal_blocked>",
        objective.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goal::test_support::TempGoalStore;
    use crate::goal::{
        active_goal_for_session, advance_goal_round, clear_goal, continuation_goal_for_run,
        create_goal, get_goal, update_goal, CreateGoalArgs, GetGoalArgs, GoalActivation,
        UpdateGoalAction, UpdateGoalArgs, GOAL_BLOCK_MIN_ROUNDS,
    };

    fn goal_with(phase: &str, blocked_reason: Option<&str>) -> GoalData {
        GoalData {
            id: "goal-wrapup-unit".to_string(),
            session_id: "wrapup-unit".to_string(),
            objective: "把报告写完并给出结论".to_string(),
            phase: phase.to_string(),
            activation: GoalActivation::Disarmed,
            revision: 3,
            rounds_started: 2,
            max_goal_rounds: 10,
            blocked_reason: blocked_reason.map(str::to_string),
        }
    }

    fn terminal_goal(session: &str, action: UpdateGoalAction) -> GoalData {
        let created = create_goal(CreateGoalArgs {
            session_id: Some(session.to_string()),
            objective: "把报告写完并给出结论".to_string(),
            max_goal_rounds: Some(10),
        })
        .expect("create_goal")
        .goal;
        // O5（2026-11-27）：`blocked` 现有最小轮次门槛（`GOAL_BLOCK_MIN_ROUNDS`）。本辅助函数
        // 同时服务 `complete` 与 `blocked`，统一先推到门槛——对 `complete` 无副作用（只关心
        // 终态相位），对 `blocked` 是满足新硬约束的最小前提。
        for _ in 0..GOAL_BLOCK_MIN_ROUNDS {
            advance_goal_round(&created.id).expect("advance to block threshold");
        }
        update_goal(UpdateGoalArgs {
            session_id: Some(session.to_string()),
            goal_id: created.id,
            revision: created.revision,
            action,
            objective: None,
            blocked_reason: Some("上游模型连续失败".to_string()),
            max_goal_rounds: None,
        })
        .expect("update_goal terminal action")
        .goal
    }

    #[test]
    fn complete_wrapup_asks_for_summary_and_forbids_new_round() {
        let text =
            wrapup_for_goal(&goal_with("completed", None)).expect("completed 必须有收尾指令");
        assert!(text.starts_with("<goal_complete>"), "{text}");
        assert!(text.ends_with("</goal_complete>"), "{text}");
        assert!(
            text.contains("把报告写完并给出结论"),
            "必须点名目标：{text}"
        );
        assert!(text.contains("总结"), "必须要求总结：{text}");
        assert!(
            text.contains("不要再调用工具"),
            "必须禁止本轮继续调工具：{text}"
        );
        assert!(
            text.contains("终态后不再续轮"),
            "必须声明不再发起新轮：{text}"
        );
    }

    #[test]
    fn blocked_wrapup_carries_reason_and_completed_parts() {
        let text = wrapup_for_goal(&goal_with("blocked", Some("上游模型连续失败")))
            .expect("blocked 必须有收尾指令");
        assert!(text.starts_with("<goal_blocked>"), "{text}");
        assert!(text.ends_with("</goal_blocked>"), "{text}");
        assert!(
            text.contains("上游模型连续失败"),
            "必须带上阻塞原因：{text}"
        );
        assert!(
            text.contains("已经完成的部分"),
            "必须交代已完成部分：{text}"
        );
        assert!(text.contains("不要再调用工具"), "{text}");
        assert!(text.contains("终态后不再续轮"), "{text}");

        // 缺理由（旧数据 / 直接改库）不得产出一句半截文案。
        let legacy = wrapup_for_goal(&goal_with("blocked", None)).expect("blocked 仍需收尾");
        assert!(legacy.contains("未提供具体原因"), "{legacy}");
    }

    #[test]
    fn non_terminal_phases_have_no_wrapup() {
        assert!(wrapup_for_goal(&goal_with("active", None)).is_none());
        assert!(wrapup_for_goal(&goal_with("paused", None)).is_none());
        assert!(is_terminal_phase("completed"));
        assert!(is_terminal_phase("blocked"));
        assert!(!is_terminal_phase("active"));
    }

    /// B129 的硬断言：真实写入路径把目标标成终态后，宿主续轮读面**不再命中**。
    ///
    /// 两个宿主循环（control_plane ×2、TUI）每轮都调用这两个读面之一；返回 `None`
    /// 即「终态后不再发起新轮」。
    #[test]
    fn terminal_goal_is_invisible_to_continuation_reads() {
        let _guard = crate::goal::test_support::lock();
        let _store = TempGoalStore::install("terminal-reads");

        let completed_session = "wrapup-unit-completed";
        terminal_goal(completed_session, UpdateGoalAction::Complete);
        assert!(
            active_goal_for_session(completed_session).is_none(),
            "completed 目标不得被续轮读面命中"
        );
        assert!(
            continuation_goal_for_run(Some(completed_session)).is_none(),
            "completed 后宿主不得再发起 goal 轮"
        );

        let blocked_session = "wrapup-unit-blocked";
        terminal_goal(blocked_session, UpdateGoalAction::Blocked);
        assert!(
            active_goal_for_session(blocked_session).is_none(),
            "blocked 目标不得被续轮读面命中"
        );
        assert!(
            continuation_goal_for_run(Some(blocked_session)).is_none(),
            "blocked 后宿主不得再发起 goal 轮"
        );

        // 对照：clear 之前记录确实存在（否则上面的 None 可能只是「没建成功」）。
        assert!(get_goal(GetGoalArgs {
            session_id: Some(completed_session.to_string())
        })
        .expect("get_goal")
        .goal
        .is_some());
        clear_goal(completed_session).expect("clear completed");
        assert!(active_goal_for_session(completed_session).is_none());
    }
}
